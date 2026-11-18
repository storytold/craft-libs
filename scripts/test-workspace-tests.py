#!/usr/bin/env python3
"""Exercise the native workflow's command selection without compiling Rust."""
import os
import re
import subprocess
import tomllib
import unittest
from unittest.mock import patch
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github/workflows/workspace.yml"


def block(text, name):
    match = re.search(r"^      " + re.escape(name) + r": \|\n((?:        .*\n|\n)+)", text, re.M)
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
TEST_SOURCE = """
cargo nextest run --workspace --all-features --locked --profile ci
cargo test --workspace --doc --all-features --locked
cargo nextest run -p craft-image-codecs -p craft-text --locked --profile ci
"""
WINDOWS_SETUP = """powershell.exe -NoProfile -Command '$ErrorActionPreference = "Stop"; Start-Service Spooler; Add-PrinterDriver -Name "Generic / Text Only"'"""
EXPECTED_SOURCE = {
    "lint-command": LINT_SOURCE,
    "command": TEST_SOURCE,
    "windows-command": WINDOWS_SETUP + "\n" + TEST_SOURCE,
}


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
case "$BLOCK" in
  lint-command)
""" + LINT_SOURCE + """
    ;;
  command)
""" + TEST_SOURCE + """
    ;;
  *) exit 29 ;;
esac
"""


def commands(os_name, fail="", name="command", source=None):
    validate_source(WORKFLOW.read_text() if source is None else source, name)
    return subprocess.run(
        ["bash", "-euo", "pipefail", "-c", HARNESS],
        env={**os.environ, "RUNNER_OS": os_name, "FAIL": fail, "BLOCK": name},
        text=True, capture_output=True, check=False,
    )


COMMON = [
    "fmt --check",
    "clippy --workspace --all-targets --locked -- -D warnings",
    "clippy --workspace --all-targets --all-features --locked -- -D warnings",
]
TESTS = [
    "nextest run --workspace --all-features --locked --profile ci",
    "test --workspace --doc --all-features --locked",
    "nextest run -p craft-image-codecs -p craft-text --locked --profile ci",
]


class WorkspaceTests(unittest.TestCase):
    def test_every_leg_lints_with_both_feature_sets(self):
        # Windows skips only rustfmt: one call with every source root of the workspace is longer
        # than a Windows command line may be (os error 206), and Linux and macOS check the same
        # formatting.
        for os_name, expected in (("Linux", COMMON), ("macOS", COMMON), ("Windows", COMMON[1:])):
            with self.subTest(os=os_name):
                result = commands(os_name, name="lint-command")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.splitlines(), expected)

    def test_linux_and_macos_run_one_feature_set_with_doctests(self):
        for os_name in ("Linux", "macOS"):
            with self.subTest(os=os_name):
                result = commands(os_name)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.splitlines(), TESTS)

    def test_each_stage_failure_remains_fatal(self):
        for name, stages in (("lint-command", COMMON), ("command", TESTS)):
            for index, command in enumerate(stages):
                with self.subTest(block=name, command=command):
                    result = commands("Linux", command, name)
                    self.assertEqual(result.returncode, 17, result.stderr)
                    self.assertEqual(result.stdout.splitlines(), stages[:index + 1])

    def test_windows_runs_the_same_tests(self):
        validate_source(WORKFLOW.read_text(), "windows-command")
        source = block(WORKFLOW.read_text(), "windows-command")
        cargo = [line.strip() for line in source.splitlines() if line.strip().startswith("cargo ")]
        self.assertEqual(cargo, ["cargo " + command for command in TESTS])

    def test_workflow_source_is_rejected_before_starting_any_process(self):
        source = WORKFLOW.read_text()
        for payload in ("cargo fmt --check; echo injected", "cargo fmt $(echo injected)",
                        "cargo fmt --check | cat", "cargo fmt --check > injected",
                        "echo injected", "cargo fmt --check\n        touch injected"):
            with self.subTest(payload=payload), patch("subprocess.run") as child:
                changed = source.replace("cargo fmt --check", payload, 1)
                with self.assertRaisesRegex(AssertionError, "unexpected command source"):
                    commands("Linux", name="lint-command", source=changed)
                child.assert_not_called()

    def test_added_or_missing_stage_is_rejected_before_execution(self):
        source = WORKFLOW.read_text()
        for replacement in ("", "cargo fmt --check\n        cargo fmt --check"):
            with self.subTest(replacement=replacement), patch("subprocess.run") as child:
                with self.assertRaisesRegex(AssertionError, "unexpected command source"):
                    commands("Linux", name="lint-command",
                             source=source.replace("cargo fmt --check", replacement, 1))
                child.assert_not_called()

    def test_ci_profile_keeps_its_failure_policy_without_retries_or_filtering(self):
        config = tomllib.loads((ROOT / ".config/nextest.toml").read_text())
        self.assertIs(config["profile"]["ci"]["fail-fast"], False)
        for profile in config["profile"].values():
            self.assertNotIn("default-filter", profile)
            self.assertNotIn("retries", profile)
        self.assertEqual(config["profile"]["ci"]["slow-timeout"], {"period": "60s"})

    def test_unix_resource_groups_are_serial_and_leave_windows_alone(self):
        config = tomllib.loads((ROOT / ".config/nextest.toml").read_text())
        overrides = config["profile"]["ci"]["overrides"]
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
        self.assertIn('os: \'["ubuntu-latest","windows-latest","macos-latest"]\'', source)
        self.assertIn("  msrv:", source)
        self.assertIn("  wasm:", source)
        self.assertIn("python3 scripts/test-workspace-tests.py", block(source, "lint-command"))


if __name__ == "__main__":
    unittest.main()

