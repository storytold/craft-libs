#!/usr/bin/env bash
# The Workspace gate's test steps (.github/workflows/workspace.yml), one per argument:
#
#   run      one leg builds and runs every test: each macOS and Windows leg, and the Linux leg when
#            the selection reaches no replay-audit crate
#   archive  the Linux build job (gate.yml `archive-command`) builds once and writes one nextest
#            archive per shard: 1 every selected test outside the replay audit's crates and the
#            crates shard 2 also takes, 2 those crates, 3 the default-feature crate
#   shard    a Linux shard leg runs its archive ($GATE_SHARD); shard 2 runs it plainly, then the
#            audit crates' tests again with the audit on
#   doctest  the doctests, in the build job after the archives (`after-archive-command`)
#   audit    prints whether the selection reaches a replay-audit crate (`true` or `false`)
#   full     the nightly run: every test, at the full sweep and proptest counts nightly.yml leaves set
#
# CRAFT_TESTS (all, some, none) and CRAFT_TEST_FILTER are the platforms job's selection;
# CRAFT_TEST_SKIP is its list of members whose tests do not run.
set -euo pipefail

# Every build passes the same profile overrides, so the steps of one leg share their build and
# match local `tools/cl` builds (.config/cargo-dev.toml); .config/cargo-ci.toml adds the gate's own.
config=(--config .config/cargo-dev.toml --config .config/cargo-ci.toml)
# Every test once, with every feature. Examples are not built, except the two whose tests run;
# clippy in the lint leg covers the rest. Every build is the whole workspace's: the filter narrows
# only what runs.
all_features=(--workspace --all-features --locked --profile ci "${config[@]}")
targets=(--lib --bins --tests)
examples=(-p craft-aep -p craft-video --example stubicc --example h264_h264enc_bench)
# The crate whose tests check behaviour with a feature off, which an all-features build cannot reach.
# craft-text is not here: its tests all need features, and its craft-ui dev-dependency would build
# the whole UI stack a second time with default features.
default_features=(-p craft-image-codecs --locked --profile ci "${config[@]}")
# The replay audit (craft-libs#2336): these crates' tests again, with every memoized view that would
# replay also run for real and compared with its replay.
audit_packages=(craft-ui-contract craft-ui-widgets craft-ui-panels craft-ui-patterns craft-ui-shell craft-ui-gallery craft-menu)
audit_crates=$(printf ' | package(%s)' "${audit_packages[@]}")
audit_crates=${audit_crates:3}
# Crates outside the audit whose tests shard 2 runs too (plainly only), so that shards 1 and 2 end
# together: shard 1 holds the other UI crates and every non-UI test.
shard_2_packages=(craft-draw)
shard_2_crates=$audit_crates$(printf ' | package(%s)' "${shard_2_packages[@]}")
filter=${CRAFT_TEST_FILTER:-all()}
# Shard 1 leaves the audit crates and shard 2's own to shard 2, which runs them plainly and then
# the audit crates with the audit on.
plain_filter="($filter) & !($shard_2_crates)"
shard_2_filter="($filter) & ($shard_2_crates)"

doctests() {
  # Whole-workspace: `cargo test -p <selected>` would resolve features over the selected crates
  # only and rebuild the dependencies whose features differ.
  cargo test --workspace --doc --all-features --locked "${config[@]}"
}

case "${1:-}" in
  run)
    if [ "${CRAFT_TESTS:-all}" = none ]; then
      echo "No crate changed: no tests to run."
      exit 0
    fi
    if [ "${RUNNER_OS:-}" = Windows ]; then
      # shellcheck disable=SC2016 # $ErrorActionPreference is PowerShell's
      powershell.exe -NoProfile -Command '$ErrorActionPreference = "Stop"; Start-Service Spooler; Add-PrinterDriver -Name "Generic / Text Only"'
    fi
    cargo nextest run "${all_features[@]}" -E "$filter" --no-tests=warn "${targets[@]}"
    cargo nextest run "${examples[@]}" --all-features --locked --profile ci "${config[@]}" -E "$filter" --no-tests=warn
    doctests
    cargo nextest run "${default_features[@]}" -E "$filter" --no-tests=warn "${targets[@]}"
    ;;
  archive)
    for shard in 1 2 3; do mkdir -p "$NEXTEST_ARCHIVE_DIR/$shard"; done
    # The examples' two packages join the one build here, so their tests are in shard 1's archive.
    cargo nextest archive "${all_features[@]}" "${targets[@]}" --example stubicc --example h264_h264enc_bench \
      -E "$plain_filter" --archive-file "$NEXTEST_ARCHIVE_DIR/1/$NEXTEST_ARCHIVE_NAME"
    # The same build, so nothing compiles again; only shard 2's crates' binaries.
    cargo nextest archive "${all_features[@]}" "${targets[@]}" --example stubicc --example h264_h264enc_bench \
      -E "$shard_2_filter" --archive-file "$NEXTEST_ARCHIVE_DIR/2/$NEXTEST_ARCHIVE_NAME"
    cargo nextest archive "${default_features[@]}" "${targets[@]}" \
      -E "$filter" --archive-file "$NEXTEST_ARCHIVE_DIR/3/$NEXTEST_ARCHIVE_NAME"
    ;;
  shard)
    run=(cargo nextest run --archive-file "$NEXTEST_ARCHIVE_FILE" --workspace-remap . --profile ci --no-tests=warn)
    case "${GATE_SHARD:-}" in
      1 | 3) "${run[@]}" ;;
      2) "${run[@]}"; CRAFT_UI_AUDIT_REPLAY=1 "${run[@]}" -E "$audit_crates" ;;
      *) echo "unknown shard: '${GATE_SHARD:-}'" >&2; exit 2 ;;
    esac
    ;;
  doctest)
    doctests
    ;;
  audit)
    case "${CRAFT_TESTS:-all}" in
      none) echo false ;;
      all) echo true ;;
      *)
        reached=false
        for package in "${audit_packages[@]}"; do
          if [[ " ${CRAFT_TEST_SKIP:-} " != *" $package "* ]]; then reached=true; fi
        done
        echo "$reached"
        ;;
    esac
    ;;
  full)
    cargo nextest run "${all_features[@]}" --no-tests=warn "${targets[@]}"
    cargo nextest run "${examples[@]}" --all-features --locked --profile ci "${config[@]}" --no-tests=warn
    ;;
  *)
    echo "usage: $0 run|archive|shard|doctest|audit|full" >&2
    exit 2
    ;;
esac
