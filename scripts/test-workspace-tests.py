#!/usr/bin/env python3
"""Exercise the native workflow's command selection without compiling Rust."""
import os
import re
import subprocess
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github/workflows/workspace.yml"


def block(text, name):
    match = re.search(r"^      " + re.escape(name) + r": \|\n((?:        .*\n|\n)+)", text, re.M)
    if match is None:
        raise AssertionError(f"missing command block: {name}")
    return "\n".join(line[8:] for line in match[1].splitlines())


def commands(os_name, fail=""):
    # Functions avoid executing any caller-supplied cargo command or changing the workspace.
    # The real bash -euo pipefail semantics still apply, including failures inside a branch.
    stub = """
cargo() {
  printf '%s\\n' "$*"
  if [ -n "$FAIL" ] && [ "$*" = "$FAIL" ]; then return 17; fi
}
python3() { :; }
"""
    return subprocess.run(
        ["bash", "-euo", "pipefail", "-c", stub + block(WORKFLOW.read_text(), "command")],
        env={**os.environ, "RUNNER_OS": os_name, "FAIL": fail},
        text=True, capture_output=True, check=False,
    )


COMMON = [
    "fmt --check",
    "clippy --workspace --all-targets --locked -- -D warnings",
    "clippy --workspace --all-targets --all-features --locked -- -D warnings",
]
LINUX = [
    "build --workspace --examples --locked",
    "nextest run --workspace --locked --profile ci-linux",
    "test --workspace --doc --locked",
    "build --workspace --examples --all-features --locked",
    "nextest run --workspace --all-features --locked --profile ci-linux",
    "test --workspace --doc --all-features --locked",
]


class WorkspaceTests(unittest.TestCase):
    def test_linux_keeps_both_feature_sets_examples_and_doctests(self):
        result = commands("Linux")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), COMMON + LINUX)

    def test_macos_keeps_cargo_test(self):
        result = commands("macOS")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), COMMON + [
            "test --workspace --locked", "test --workspace --all-features --locked"])

    def test_each_linux_stage_failure_remains_fatal(self):
        for index, command in enumerate(COMMON + LINUX):
            with self.subTest(command=command):
                result = commands("Linux", command)
                self.assertEqual(result.returncode, 17, result.stderr)
                self.assertEqual(result.stdout.splitlines(), (COMMON + LINUX)[:index + 1])

    def test_windows_keeps_its_existing_profile_and_doctests(self):
        source = block(WORKFLOW.read_text(), "windows-command")
        cargo = [line.strip() for line in source.splitlines() if line.strip().startswith("cargo ")]
        self.assertEqual(cargo, ["cargo " + command for command in COMMON + [
            "nextest run --workspace --locked --profile ci",
            "test --workspace --doc --locked",
            "nextest run --workspace --all-features --locked --profile ci",
            "test --workspace --doc --all-features --locked",
        ]])

    def test_linux_inherits_no_retries_no_filtering_and_existing_failure_policy(self):
        config = tomllib.loads((ROOT / ".config/nextest.toml").read_text())
        self.assertEqual(config["profile"]["ci-linux"]["inherits"], "ci")
        self.assertIs(config["profile"]["ci"]["fail-fast"], False)
        for profile in config["profile"].values():
            self.assertNotIn("default-filter", profile)
            self.assertNotIn("retries", profile)
        self.assertEqual(config["profile"]["ci"]["slow-timeout"], {"period": "60s"})

    def test_linux_resource_groups_are_serial_and_do_not_change_windows(self):
        config = tomllib.loads((ROOT / ".config/nextest.toml").read_text())
        overrides = config["profile"]["ci-linux"]["overrides"]
        self.assertEqual(overrides, [
            {"filter": "package(craft-ui) | package(craft-keymap) | package(craft-update)",
             "test-group": "gpu"},
            {"filter": "package(craft-video) & kind(test)", "test-group": "linux-video-fixtures"},
        ])
        for override in overrides:
            self.assertEqual(config["test-groups"][override["test-group"]]["max-threads"], 1)
        self.assertNotIn("overrides", config["profile"]["ci"])

    def test_workflow_stays_shared_and_keeps_platform_gates(self):
        source = WORKFLOW.read_text()
        self.assertIn("uses: storytold/craft-common-actions/.github/workflows/gate.yml@main", source)
        self.assertIn('os: \'["ubuntu-latest","windows-latest","macos-latest"]\'', source)
        self.assertIn("  msrv:", source)
        self.assertIn("  wasm:", source)
        self.assertIn("python3 scripts/test-workspace-tests.py", block(source, "command"))


if __name__ == "__main__":
    unittest.main()

