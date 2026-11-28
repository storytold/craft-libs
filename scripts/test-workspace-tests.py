#!/usr/bin/env python3
"""Exercise the Workspace gate's lint block and test steps without compiling Rust."""
import os
import re
import tempfile
import subprocess
import tomllib
import unittest
from unittest.mock import patch
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github/workflows/workspace.yml"


def block(text, name):
    # The Linux call names each shared block with a YAML anchor (`command: &command |`), and the
    # macOS and Windows call refers to it, so this first block is the one every leg runs.
    match = re.search(r"^      " + re.escape(name) + r": (?:&[\w-]+ )?\|\n((?:        .*\n|\n)+)", text, re.M)
    if match is None:
        raise AssertionError(f"missing command block: {name}")
    return "\n".join(line[8:] for line in match[1].splitlines())


# Only these literal programs may be exercised. Workflow text is compared, never executed.
LINT_SOURCE = """
if [ "$RUNNER_OS" = Linux ]; then
  python3 scripts/test-workspace-concurrency.py
  python3 scripts/test-workspace-tests.py
fi
if [ "$RUNNER_OS" != Windows ]; then
  cargo fmt --check
fi
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
"""
EXPECTED_SOURCE = {"lint-command": LINT_SOURCE}
SCRIPT = ROOT / "scripts/ci-workspace-tests.sh"
NIGHTLY = ROOT / ".github/workflows/nightly.yml"


def source_lines(source):
    return [line.strip() for line in source.splitlines()
            if line.strip() and not line.lstrip().startswith("#")]


def validate_source(text, name):
    if name not in EXPECTED_SOURCE:
        raise AssertionError(f"unsupported command block: {name}")
    if source_lines(block(text, name)) != source_lines(EXPECTED_SOURCE[name]):
        raise AssertionError(f"unexpected command source: {name}")


# The child shell only receives this checked-in literal, never bytes read from the workflow.
HARNESS = """
cargo() {
  printf '%s\\n' "$*"
  if [ -n "$FAIL" ] && [ "$*" = "$FAIL" ]; then return 17; fi
}
python3() { :; }
""" + LINT_SOURCE


def lint(os_name, fail="", source=None):
    validate_source(WORKFLOW.read_text() if source is None else source, "lint-command")
    return subprocess.run(
        ["bash", "-euo", "pipefail", "-c", HARNESS],
        env={**os.environ, "RUNNER_OS": os_name, "FAIL": fail},
        text=True, capture_output=True, check=False,
    )


# Stand-ins for the programs the test steps start: each prints its arguments (cargo without its
# name, as above), and cargo fails with 17 on the command named in $FAIL.
FAKE_CARGO = """#!/usr/bin/env bash
line="$*"
if [ -n "${CRAFT_UI_AUDIT_REPLAY:-}" ]; then line="AUDIT $line"; fi
printf '%s\\n' "$line"
if [ -n "${FAIL:-}" ] && [ "$line" = "$FAIL" ]; then exit 17; fi
"""
FAKE_POWERSHELL = """#!/usr/bin/env bash
echo powershell.exe
"""


def step(name, os_name="Linux", fail="", tests="all", test_filter="all()", **env):
    with tempfile.TemporaryDirectory() as tmp:
        for program, text in (("cargo", FAKE_CARGO), ("powershell.exe", FAKE_POWERSHELL)):
            path = Path(tmp) / program
            path.write_text(text)
            path.chmod(0o755)
        archives = Path(tmp) / "archives"
        result = subprocess.run(
            ["bash", str(SCRIPT), name],
            env={**os.environ, "PATH": f"{tmp}:{os.environ['PATH']}", "RUNNER_OS": os_name, "FAIL": fail,
                 "CRAFT_TESTS": tests, "CRAFT_TEST_FILTER": test_filter,
                 "NEXTEST_ARCHIVE_DIR": str(archives), "NEXTEST_ARCHIVE_NAME": "a.tar.zst", **env},
            text=True, capture_output=True, check=False, cwd=ROOT,
        )
        made = sorted(p.name for p in archives.iterdir()) if archives.exists() else []
        return result, made


COMMON = [
    "fmt --check",
    "clippy --workspace --all-targets --locked -- -D warnings",
    "clippy --workspace --all-targets --all-features --locked -- -D warnings",
]
CONFIG = "--config .config/cargo-dev.toml --config .config/cargo-ci.toml"
ALL = f"--workspace --all-features --locked --profile ci {CONFIG}"
DEFAULT = f"-p craft-image-codecs --locked --profile ci {CONFIG}"
TARGETS = "--lib --bins --tests"
EXAMPLES = "--example stubicc --example h264_h264enc_bench"
AUDIT_PACKAGES = ("craft-ui-contract", "craft-ui-widgets", "craft-ui-panels", "craft-ui-patterns",
                  "craft-ui-shell", "craft-ui-gallery", "craft-menu")
AUDIT = " | ".join(f"package({name})" for name in AUDIT_PACKAGES)
SHARD_2 = AUDIT + " | package(craft-draw)"
DOCTESTS = f"test --workspace --doc --all-features --locked {CONFIG}"


def run_commands(test_filter="all()"):
    return [
        f"nextest run {ALL} -E {test_filter} --no-tests=warn {TARGETS}",
        f"nextest run -p craft-aep -p craft-video {EXAMPLES} --all-features --locked --profile ci {CONFIG} "
        f"-E {test_filter} --no-tests=warn",
        DOCTESTS,
        f"nextest run {DEFAULT} -E {test_filter} --no-tests=warn {TARGETS}",
    ]


def archive_commands(directory, test_filter="all()"):
    return [
        f"nextest archive {ALL} {TARGETS} {EXAMPLES} -E ({test_filter}) & !({SHARD_2}) "
        f"--archive-file {directory}/1/a.tar.zst",
        f"nextest archive {ALL} {TARGETS} {EXAMPLES} -E ({test_filter}) & ({SHARD_2}) "
        f"--archive-file {directory}/2/a.tar.zst",
        f"nextest archive {DEFAULT} {TARGETS} -E {test_filter} --archive-file {directory}/3/a.tar.zst",
    ]


def archive_dir(result):
    # The harness's temporary directory, read back from the first archive path.
    return re.search(r"--archive-file (\S+)/1/a\.tar\.zst", result.stdout)[1]


class WorkspaceTests(unittest.TestCase):
    def test_every_leg_lints_with_both_feature_sets(self):
        # Windows skips only rustfmt: one call with every source root of the workspace is longer
        # than a Windows command line may be (os error 206), and Linux and macOS check the same
        # formatting.
        for os_name, expected in (("Linux", COMMON), ("macOS", COMMON), ("Windows", COMMON[1:])):
            with self.subTest(os=os_name):
                result = lint(os_name)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.splitlines(), expected)

    def test_one_leg_runs_one_feature_set_with_doctests_and_the_feature_off_crates(self):
        for os_name, expected in (("Linux", run_commands()), ("macOS", run_commands()),
                                  ("Windows", ["powershell.exe"] + run_commands())):
            with self.subTest(os=os_name):
                result, _ = step("run", os_name)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.splitlines(), expected)

    def test_a_selection_filters_every_nextest_run_and_none_runs_nothing(self):
        selected = "package(=craft-geom) | package(=craft-svg)"
        result, _ = step("run", tests="some", test_filter=selected)
        self.assertEqual(result.returncode, 0, result.stderr)
        # The doctests stay whole-workspace; every nextest run carries the filter.
        self.assertEqual(result.stdout.splitlines(), run_commands(selected))
        result, _ = step("run", tests="none", test_filter="none()")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), ["No crate changed: no tests to run."])

    def test_the_build_job_writes_one_archive_per_shard_from_one_build(self):
        selected = "package(=craft-ui-widgets) | package(=craft-svg)"
        result, made = step("archive", tests="some", test_filter=selected)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(made, ["1", "2", "3"])
        self.assertEqual(result.stdout.splitlines(), archive_commands(archive_dir(result), selected))
        # The audit's archive is the same build as shard 1's: only the filter and the file differ.
        first, audit = (re.sub(r" -E .* --archive-file .*", "", line) for line in result.stdout.splitlines()[:2])
        self.assertEqual(first, audit)
        result, _ = step("doctest")
        self.assertEqual(result.stdout.splitlines(), [DOCTESTS])

    def test_every_build_passes_the_profile_overrides(self):
        # A step without them would build the workspace a second time.
        lines = []
        for name in ("run", "archive", "doctest", "full"):
            result, _ = step(name)
            self.assertEqual(result.returncode, 0, result.stderr)
            lines += result.stdout.splitlines()
        for line in lines:
            with self.subTest(command=line):
                self.assertIn(CONFIG, line)

    def test_each_shard_runs_its_archive_and_shard_2_also_audits(self):
        expected = "nextest run --archive-file /a/b.tar.zst --workspace-remap . --profile ci --no-tests=warn"
        # Shard 2 audits only the audit crates of its archive.
        for shard, lines in (("1", [expected]), ("2", [expected, f"AUDIT {expected} -E {AUDIT}"]), ("3", [expected])):
            with self.subTest(shard=shard):
                result, _ = step("shard", GATE_SHARD=shard, NEXTEST_ARCHIVE_FILE="/a/b.tar.zst")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.splitlines(), lines)
        result, _ = step("shard", GATE_SHARD="4", NEXTEST_ARCHIVE_FILE="/a/b.tar.zst")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(result.stdout, "")

    def test_the_audit_runs_when_the_selection_reaches_an_audit_crate(self):
        everything_but = " ".join(AUDIT_PACKAGES)
        for tests, skip, expected in (("none", "", "false"), ("all", "", "true"),
                                      ("some", everything_but + " craft-svg", "false"),
                                      ("some", everything_but.replace("craft-menu", "craft-svg"), "true")):
            with self.subTest(tests=tests, skip=skip):
                result, _ = step("audit", tests=tests, CRAFT_TEST_SKIP=skip)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.strip(), expected)

    def test_each_stage_failure_remains_fatal(self):
        for index, command in enumerate(COMMON):
            with self.subTest(block="lint-command", command=command):
                result = lint("Linux", command)
                self.assertEqual(result.returncode, 17, result.stderr)
                self.assertEqual(result.stdout.splitlines(), COMMON[:index + 1])
        for index, command in enumerate(run_commands()):
            with self.subTest(step="run", command=command):
                result, _ = step("run", fail=command)
                self.assertEqual(result.returncode, 17, result.stderr)
                self.assertEqual(result.stdout.splitlines(), run_commands()[:index + 1])
        for index in range(3):
            with self.subTest(step="archive", stage=index):
                with tempfile.TemporaryDirectory() as tmp:
                    command = archive_commands(tmp + "/archives")[index]
                    result, _ = step("archive", fail=command, NEXTEST_ARCHIVE_DIR=tmp + "/archives")
                    self.assertEqual(result.returncode, 17, result.stderr)
                    self.assertEqual(result.stdout.splitlines(), archive_commands(tmp + "/archives")[:index + 1])

    def test_workflow_source_is_rejected_before_starting_any_process(self):
        source = WORKFLOW.read_text()
        for payload in ("cargo fmt --check; echo injected", "cargo fmt $(echo injected)",
                        "cargo fmt --check | cat", "cargo fmt --check > injected",
                        "echo injected", "cargo fmt --check\n        touch injected"):
            with self.subTest(payload=payload), patch("subprocess.run") as child:
                changed = source.replace("cargo fmt --check", payload, 1)
                with self.assertRaisesRegex(AssertionError, "unexpected command source"):
                    lint("Linux", source=changed)
                child.assert_not_called()

    def test_added_or_missing_stage_is_rejected_before_execution(self):
        source = WORKFLOW.read_text()
        for replacement in ("", "cargo fmt --check\n        cargo fmt --check"):
            with self.subTest(replacement=replacement), patch("subprocess.run") as child:
                with self.assertRaisesRegex(AssertionError, "unexpected command source"):
                    lint("Linux", source=source.replace("cargo fmt --check", replacement, 1))
                child.assert_not_called()

    def test_profile_overrides_name_only_workspace_members(self):
        # Cargo warns on every command about an override for a package outside the workspace.
        members = {tomllib.loads(path.read_text())["package"]["name"] for path in ROOT.glob("crates/*/Cargo.toml")}
        for name in ("cargo-dev.toml", "cargo-ci.toml"):
            with self.subTest(file=name):
                config = tomllib.loads((ROOT / ".config" / name).read_text())
                self.assertEqual(set(config), {"profile"})
                self.assertEqual(set(config["profile"]), {"dev"})
                packages = set(config["profile"]["dev"].get("package", {})) - {"*"}
                self.assertLessEqual(packages, members)

    def test_ci_profile_keeps_its_failure_policy_without_retries_or_filtering(self):
        config = tomllib.loads((ROOT / ".config/nextest.toml").read_text())
        self.assertIs(config["profile"]["ci"]["fail-fast"], False)
        for profile in config["profile"].values():
            self.assertNotIn("default-filter", profile)
            self.assertNotIn("retries", profile)
        self.assertEqual(config["profile"]["ci"]["slow-timeout"], {"period": "60s"})

    def test_unix_resource_groups_are_serial_and_leave_windows_alone(self):
        config = tomllib.loads((ROOT / ".config/nextest.toml").read_text())
        # The priority overrides (slowest tests first) set no group or platform; only the group
        # overrides are the resource audit's.
        overrides = [o for o in config["profile"]["ci"]["overrides"] if "priority" not in o]
        for o in config["profile"]["ci"]["overrides"]:
            if "priority" in o:
                self.assertEqual(set(o), {"filter", "priority"})
        self.assertEqual(overrides, [
            {"platform": "cfg(unix)",
             "filter": "package(craft-ui) | package(craft-keymap) | package(craft-update)",
             "test-group": "gpu"},
            {"platform": "cfg(unix)", "filter": "package(craft-video) & kind(test)",
             "test-group": "video-integration"},
        ])
        for override in overrides:
            self.assertEqual(config["test-groups"][override["test-group"]]["max-threads"], 1)

    def test_workflow_stays_shared_and_keeps_platform_gates(self):
        source = WORKFLOW.read_text()
        self.assertIn("uses: storytold/craft-common-actions/.github/workflows/gate.yml@main", source)
        self.assertIn('os: \'["ubuntu-latest"]\'', source)
        self.assertIn('os: \'["windows-latest","macos-latest"]\'', source)
        self.assertIn("if: needs.platforms.outputs.platform == 'true'", source)
        # Pull requests run Linux only, unless labelled full-ci; every other event runs every leg.
        self.assertIn("&& (github.event_name != 'pull_request'"
                      " || contains(github.event.pull_request.labels.*.name, 'full-ci'))", source)
        # Every leg reads the test selection of the platforms job.
        self.assertIn("  native:\n    name: Workspace\n    needs: platforms\n", source)
        self.assertIn("          select-tests: true\n", source)
        self.assertIn("        CRAFT_TESTS=${{ needs.platforms.outputs.tests }}\n"
                      "        CRAFT_TEST_FILTER=${{ needs.platforms.outputs.filter }}\n", source)
        self.assertIn("      env: *env\n", source)
        # The macOS and Windows legs run the Linux leg's blocks and steps, not copies of them.
        self.assertIn("      lint-command: &lint |\n", source)
        self.assertIn("      command: &command bash scripts/ci-workspace-tests.sh run\n", source)
        self.assertIn("      lint-command: *lint\n", source)
        self.assertIn("      command: *command\n", source)
        self.assertNotIn("windows-command", source)
        # Only the Linux leg splits into a build job and shards, and only for the audit.
        self.assertEqual(source.count("test-shards:"), 1)
        self.assertIn("test-shards: ${{ needs.platforms.outputs.audit == 'true' && 3 || 0 }}", source)
        # The Linux leg's entry, which keeps workspace crates, is named after every profile override;
        # the macOS and Windows legs' dependency-only entries after the settings that reach a
        # dependency (test_profile_overrides_name_only_workspace_members).
        self.assertIn("cache-version: &cache-version v2-${{ needs.platforms.outputs.profile }}\n", source)
        self.assertIn("      cache-version: v2-${{ needs.platforms.outputs.deps-profile }}\n", source)
        self.assertIn("      deps-profile: ${{ steps.audit.outputs.deps-profile }}\n", source)
        self.assertNotIn("      cache-version: *cache-version\n", source)
        # Only the Linux leg keeps workspace crates; the cache budget has no room for the others.
        self.assertEqual(source.count("cache-workspace-crates: true"), 1)
        self.assertIn("  msrv:", source)
        self.assertIn("  wasm:", source)
        self.assertIn("python3 scripts/test-workspace-tests.py", block(source, "lint-command"))
        # The nightly run builds as the gate does.
        self.assertIn("command: bash scripts/ci-workspace-tests.sh full\n", NIGHTLY.read_text())


if __name__ == "__main__":
    unittest.main()
