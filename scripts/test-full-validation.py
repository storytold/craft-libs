#!/usr/bin/env python3
"""Exercise the full gate's command sequence without building Rust locally."""
import os
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/ci-full-validation.sh"
# Actions uses Git Bash on Windows; Python's PATH search can instead select the WSL launcher.
BASH = (str(Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "Git/bin/bash.exe")
        if sys.platform == "win32" else "bash")
COMMANDS = [
    "fmt --check",
    "clippy --workspace --all-targets --locked -- -D warnings",
    "clippy --workspace --all-targets --all-features --locked -- -D warnings",
    "test --workspace --locked",
    "test --workspace --all-features --locked",
]


# Validate the real script, but execute only this fixed, reviewed command sequence.
# Repository-file bytes must never become bash -c source in a mock-only harness.
SOURCE = r"""
set -euo pipefail
unset CRAFT_HOSTILE_DIVISOR PROPTEST_CASES
if [ "${RUNNER_OS:-}" = Windows ]; then
  powershell.exe -NoProfile -Command '$ErrorActionPreference = "Stop"; Start-Service Spooler; Add-PrinterDriver -Name "Generic / Text Only"'
fi
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
cargo test --workspace --all-features --locked
"""


def source_lines(text):
    return [line.strip() for line in text.splitlines()
            if line.strip() and not line.lstrip().startswith("#")]


def run(os_name, fail=""):
    if source_lines(SCRIPT.read_text()) != source_lines(SOURCE):
        raise AssertionError("full validation command sequence changed")
    env = dict(os.environ, RUNNER_OS=os_name, FAIL=fail,
               CRAFT_HOSTILE_DIVISOR="8", PROPTEST_CASES="64")
    mock = r"""
cargo() {
  test -z "${CRAFT_HOSTILE_DIVISOR+x}" && test -z "${PROPTEST_CASES+x}" || return 29
  printf '%s\n' "$*"
  if [ "$*" = "$FAIL" ]; then return 17; fi
}
powershell.exe() { printf 'print-queue\n'; }
"""
    return subprocess.run([BASH, "-c", mock + SOURCE], env=env,
                          capture_output=True, text=True, check=False)


class FullValidation(unittest.TestCase):
    def test_changed_source_is_rejected_without_execution(self):
        for source in [
            SOURCE + "\necho injected",
            SOURCE.replace("cargo fmt --check", "cargo fmt --check; echo injected"),
            SOURCE.replace("cargo fmt --check", "cargo fmt $(echo injected) --check"),
            SOURCE.replace("cargo test --workspace --locked", ""),
            SOURCE + "\ncargo test --workspace --locked",
        ]:
            with self.subTest(source=source):
                with patch.object(Path, "read_text", return_value=source), patch.object(subprocess, "run") as execute:
                    with self.assertRaises(AssertionError):
                        run("Linux")
                    execute.assert_not_called()

    def test_comments_and_blank_lines_do_not_change_commands(self):
        with patch.object(Path, "read_text", return_value="# harmless comment\n\n" + SOURCE):
            result = run("Linux")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(result.stdout.splitlines(), COMMANDS)

    def test_every_os_runs_both_full_configurations(self):
        for name in ["Linux", "macOS", "Windows"]:
            with self.subTest(os=name):
                p = run(name)
                self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
                self.assertEqual(p.stdout.splitlines(),
                                 (["print-queue"] if name == "Windows" else []) + COMMANDS)

    def test_every_failure_stops_the_gate(self):
        for i, command in enumerate(COMMANDS):
            with self.subTest(command=command):
                p = run("Linux", command)
                self.assertEqual(p.returncode, 17, p.stdout + p.stderr)
                self.assertEqual(p.stdout.splitlines(), COMMANDS[:i + 1])

    def test_workflow_preserves_fork_and_platform_boundaries(self):
        text = (ROOT / ".github/workflows/full-validation.yml").read_text()
        self.assertIn("os: '[\"ubuntu-latest\",\"windows-latest\",\"macos-latest\"]'", text)
        self.assertIn("select-os: '[]'", text)
        self.assertIn("contents: read", text)
        self.assertNotIn("pull_request_target", text)
        self.assertNotIn("secrets:", text)
        self.assertIn("bash scripts/ci-full-validation.sh", text)
        self.assertIn("github.event.label.name != 'full-validation' && github.run_id", text)
        self.assertIn("github.event.action != 'labeled' || github.event.label.name == 'full-validation'", text)
        self.assertNotIn("CRAFT_HOSTILE_DIVISOR=", text)
        self.assertNotIn("PROPTEST_CASES=", text)

    def test_pdf_contracts_are_separate_and_uncapped(self):
        text = (ROOT / ".github/workflows/full-validation.yml").read_text()
        pdf = text.split("  pdf-contracts:", 1)[1]
        self.assertIn("RUSTFLAGS=--cfg craft_pdf_unicode_fixtures", pdf)
        self.assertIn("unset CRAFT_HOSTILE_DIVISOR PROPTEST_CASES", pdf)
        self.assertIn("cargo test -p craft-pdf --all-features --locked", pdf)
        self.assertIn("cargo test --manifest-path crates/pdf/read-only-contract/Cargo.toml --locked", pdf)
        self.assertIn("select-os: '[]'", pdf)
        self.assertIn("windows-latest", pdf)
        self.assertIn("macos-latest", pdf)


if __name__ == "__main__":
    unittest.main()
