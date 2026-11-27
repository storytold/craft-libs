# The second round of 2026-11-27 in craft-libs

## What was merged

`next` took 13 upstream commits over `ea14416aa`, up to `a46af76c7`, as merge commit `b3e81eb46`.
No file conflicted. Against the merge base, 0 of 9 upstream files and 0 of 52 fork files were dropped,
and every file both sides touched keeps the lines both added.

## The design lint's red, which is upstream's own

Upstream `4062c6aa1` asserts a literal `4.0` in `crates/ui-panels/src/dialog/trials.rs:601`: the
`test/literal-size` rule reads it as a size in a test, which the two trial rooms already exempt
(`docking/iconic/trials.rs`, `rails/tool/trials.rs`). The fork follows that precedent rather than
editing upstream's trial: a `rules = ["test/literal-size"]` exemption in `design-lint.toml` and one
exception entry with `count = 1` in `design/exceptions/craft-ui-panels/dialog.toml`, after which the
lint reports 28 passed. Upstream's own `next` carries the same red.

## Space activates on the release, and never waits

Two findings of pull request #2239 met the same arm:

- `PressRoute::Space` emitted `Intent::Activate` on the press, so a repeated Space pressed twice and
  the declared release-time activation contract was not what the code did. It now holds the stop
  (`space_held::hold`) and activates nothing; the pass that carries the release activates the held
  record once and clears it, as a mouse release clicks, even when the focus moved in between.
- The input hook deferred a Space press that arrived with its release, so that both reach the same
  pass. On the next pass the same pair came back at the head of the queue and was deferred again: the
  release never reached the strip and every later event for that viewport waited behind it. The
  deferral is gone; the hold carries the pair instead.

Enter stays immediate, and the canvas keeps its Space.

## A failed CUPS listing is a failure, not a system without printers

`lpstat -p` reports an unreachable scheduler on stderr with a nonzero exit and no queue list. The
runner discarded the exit status, so an empty answer and a failure looked alike, the spool sent an
empty success, and `State::poll` cleared the remembered destination instead of reporting
`DiscoveryFailed`. The Unix runner now returns the exit status and stderr
(`output_within_full`), and the listing returns `None` on a failed exit unless the failure is the
supported driverless-only answer (`No destinations added`); a successful empty listing stays an empty
list, and the public `printers()` keeps its empty-list fallback for callers that cannot take `None`.

## Measured

| what | result |
|---|---|
| workspace suite | 18,729 run, 18,728 passed, 63 skipped, 1 failed |
| the failure | upstream's `craft-xlsx` stack test, section 4 of the specification |
| format, clippy | 0, 0 |
| design lint | 28 passed |
