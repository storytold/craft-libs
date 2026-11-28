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
unset PROPTEST_CASES
export CRAFT_HOSTILE_DIVISOR=1
if [[ "${RUNNER_OS:-}" = Windows ]]; then
  powershell.exe -NoProfile -Command '$ErrorActionPreference = "Stop"; Start-Service Spooler; Add-PrinterDriver -Name "Generic / Text Only"'
fi
if [[ "${RUNNER_OS:-}" = Windows ]]; then
  python scripts/ci-format-workspace.py
else
  cargo fmt --check
fi
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
  test "${CRAFT_HOSTILE_DIVISOR-}" = 1 && test -z "${PROPTEST_CASES+x}" || return 29
  printf '%s\n' "$*"
  if [ "$*" = "$FAIL" ]; then return 17; fi
}
powershell.exe() { printf 'print-queue\n'; }
python() { test "$*" = "scripts/ci-format-workspace.py" || return 31; cargo fmt --check; }
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
        for name in ["Linux", "macOS", "Windows"]:
            for i, command in enumerate(COMMANDS):
                with self.subTest(os=name, command=command):
                    p = run(name, command)
                    self.assertEqual(p.returncode, 17, p.stdout + p.stderr)
                    self.assertEqual(p.stdout.splitlines(),
                                     (["print-queue"] if name == "Windows" else []) + COMMANDS[:i + 1])

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
        self.assertNotIn("CRAFT_HOSTILE_DIVISOR=8", text)
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



class PackageFormatting(unittest.TestCase):
    @staticmethod
    def helper():
        import importlib.util
        spec = importlib.util.spec_from_file_location("ci_format_workspace", ROOT / "scripts/ci-format-workspace.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    @staticmethod
    def metadata():
        return {"workspace_members": ["id-b", "id-a"], "workspace_default_members": ["id-a"],
                "packages": [{"id": "id-a", "name": "craft-a"},
                             {"id": "id-b", "name": "craft-b"},
                             {"id": "external", "name": "not-a-member"}]}

    def test_all_workspace_members_not_only_defaults_and_no_external_dependencies(self):
        helper = self.helper()
        self.assertEqual(helper.workspace_names(self.metadata()), ["craft-a", "craft-b"])

    def test_new_package_automatically_included(self):
        helper = self.helper()
        data = self.metadata()
        data["workspace_members"].append("id-new")
        data["packages"].append({"id": "id-new", "name": "craft-new"})
        self.assertEqual(helper.workspace_names(data), ["craft-a", "craft-b", "craft-new"])

    def test_bad_inventory_fails_closed(self):
        helper = self.helper()
        for members in ([], ["id-a", "id-a"], ["missing"]):
            data = self.metadata()
            data["workspace_members"] = members
            with self.subTest(members=members), self.assertRaises((ValueError, KeyError)):
                helper.workspace_names(data)
        for name in ("--all", "craft-a;echo injected", "craft-a", ""):
            data = self.metadata()
            data["packages"][1]["name"] = name
            with self.subTest(name=name), self.assertRaises(ValueError):
                helper.workspace_names(data)

    def test_calls_cargo_with_one_package_and_no_shell(self):
        import json
        helper = self.helper()
        result = subprocess.CompletedProcess([], 0, stdout=json.dumps(self.metadata()))
        with patch.object(helper.subprocess, "run", return_value=result) as execute:
            helper.check_workspace()
        self.assertEqual([call.args[0] for call in execute.call_args_list], [
            ["cargo", "metadata", "--no-deps", "--offline", "--locked", "--format-version", "1"],
            ["cargo", "fmt", "--check", "--package", "craft-a"],
            ["cargo", "fmt", "--check", "--package", "craft-b"],
        ])
        self.assertEqual(execute.call_args_list[0].kwargs["encoding"], "utf-8")
        for call in execute.call_args_list:
            self.assertEqual(call.kwargs["cwd"], ROOT)
            self.assertTrue(call.kwargs["check"])
            self.assertNotIn("shell", call.kwargs)

    def test_metadata_failure_prevents_formatting(self):
        helper = self.helper()
        failure = subprocess.CalledProcessError(17, ["cargo", "metadata"])
        with patch.object(helper.subprocess, "run", side_effect=failure) as execute:
            with self.assertRaises(subprocess.CalledProcessError):
                helper.check_workspace()
        self.assertEqual(execute.call_count, 1)

    def test_first_format_failure_stops_later_packages(self):
        import json
        helper = self.helper()
        result = subprocess.CompletedProcess([], 0, stdout=json.dumps(self.metadata()))
        failure = subprocess.CalledProcessError(17, ["cargo", "fmt"])
        with patch.object(helper.subprocess, "run", side_effect=[result, failure]) as execute:
            with self.assertRaises(subprocess.CalledProcessError):
                helper.check_workspace()
        self.assertEqual(execute.call_count, 2)


if __name__ == "__main__":
    unittest.main()
