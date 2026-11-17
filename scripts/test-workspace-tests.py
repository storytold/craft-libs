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
TESTS = [
    "nextest run --workspace --all-features --locked --profile ci",
    "test --workspace --doc --all-features --locked",
    "nextest run -p craft-image-codecs -p craft-text --locked --profile ci",
]


class WorkspaceTests(unittest.TestCase):
    def test_linux_and_macos_run_one_feature_set_with_doctests(self):
        for os_name in ("Linux", "macOS"):
            with self.subTest(os=os_name):
                result = commands(os_name)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.splitlines(), COMMON + TESTS)

    def test_each_stage_failure_remains_fatal(self):
        for index, command in enumerate(COMMON + TESTS):
            with self.subTest(command=command):
                result = commands("Linux", command)
                self.assertEqual(result.returncode, 17, result.stderr)
                self.assertEqual(result.stdout.splitlines(), (COMMON + TESTS)[:index + 1])

    def test_windows_runs_the_same_cargo_commands(self):
        source = block(WORKFLOW.read_text(), "windows-command")
        cargo = [line.strip() for line in source.splitlines() if line.strip().startswith("cargo ")]
        self.assertEqual(cargo, ["cargo " + command for command in COMMON + TESTS])

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
        self.assertIn("python3 scripts/test-workspace-tests.py", block(source, "command"))


if __name__ == "__main__":
    unittest.main()

