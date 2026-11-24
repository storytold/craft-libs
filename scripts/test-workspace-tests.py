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
# The `platforms` job's selection: no tests when no crate changed, otherwise a nextest filter.
SELECT_GUARD = """
if [ "$CRAFT_TESTS" = none ]; then
  echo "No crate changed: no tests to run."
  exit 0
fi
"""
# Match the default-only checks before and after their feature_config_ renames.
# Legacy names are exact and scoped to their package and binary.
DEFAULT_FEATURE_TESTS = 'test(/(^|::)feature_config_[^:]*$/) | (package(=craft-image-codecs) & kind(lib) & test(/^(codecs::heif::tests::without_the_feature_heif_is_a_clear_unsupported_error)$/)) | (package(=craft-image-codecs) & binary(=it) & test(/^(decode_warnings::single_images_carry_no_warnings|detect_caps::(avif_is_readable_only_with_the_decode_feature|caps_function_matches_method|default_build_formats_all_symmetric|detect_encoded_output_of_every_format|every_enabled_format_is_symmetric_or_whitelisted|heif_is_detected_always_and_readable_only_with_the_feature|never_write_only_without_exception|sgi_requires_explicit_feature|sun_raster_disabled_by_default)|fidelity::(avif_default_build_write_unsupported|warnings_match_actual_roundtrip_behaviour)|limits::(limits_none_accepts|max_alloc_enforced_for_every_format|max_height_enforced_for_every_format|max_pixels_enforced_for_every_format|max_width_enforced_for_every_format)|malformed::(bit_flips_never_panic|byte_overwrites_in_header_never_panic|empty_input_errors_for_every_format|mutated_valid_files_never_panic|random_bytes_each_format_never_panic|random_bytes_with_magic_never_panic|truncated_at_many_offsets|truncated_files_mostly_error)|orientation::probe_size_matches_decode|roundtrip::(empty_image_rejected|one_pixel_images_roundtrip))$/)) | (package(=craft-image-codecs) & binary(=jxl_export) & test(/^(jpeg_xl_is_export_only_without_the_jxl_feature_and_detects_both_signatures)$/)) | (package(=craft-text) & kind(lib) & test(/^(hyphen::tests::language_names_and_codes|segment::tests::(breaks_are_byte_offsets_and_newlines_are_mandatory|chinese_kinsoku_keeps_punctuation_with_its_text|emergency_breaks_keep_grapheme_clusters_whole|kinsoku_adjusts_uax14_next_to_east_asian_text_only|kinsoku_keeps_closing_marks_off_line_starts_and_opening_ones_off_line_ends|kinsoku_mode_decides_whether_a_small_kana_may_start_a_line|korean_words_break_at_spaces_only|language_picks_korean_words|no_break_puts_a_closing_mark_at_a_line_start_or_an_opening_one_at_a_line_end|no_kinsoku_mode_breaks_inside_a_cluster|soft_kinsoku_lets_small_kana_and_the_prolonged_sound_mark_start_a_line|thai_without_its_feature_breaks_as_uax14))$/))'
DEFAULT_FILTER_ARGUMENT = '"($CRAFT_TEST_FILTER) & (' + DEFAULT_FEATURE_TESTS + ')"'
TEST_RUN = """
cargo nextest run --workspace --all-features --locked --profile ci -E "$CRAFT_TEST_FILTER" --no-tests=warn --lib --bins --tests
cargo nextest run -p craft-aep -p craft-video --all-features --locked --profile ci -E "$CRAFT_TEST_FILTER" --no-tests=warn --example stubicc --example h264_h264enc_bench
cargo test --workspace --doc --all-features --locked
cargo nextest run -p craft-image-codecs -p craft-text --locked --profile ci -E __DEFAULT_FILTER_ARGUMENT__ --no-tests=warn --lib --bins --tests
""".replace("__DEFAULT_FILTER_ARGUMENT__", DEFAULT_FILTER_ARGUMENT)
TEST_SOURCE = SELECT_GUARD + TEST_RUN
WINDOWS_SETUP = """powershell.exe -NoProfile -Command '$ErrorActionPreference = "Stop"; Start-Service Spooler; Add-PrinterDriver -Name "Generic / Text Only"'"""
EXPECTED_SOURCE = {
    "lint-command": LINT_SOURCE,
    "command": TEST_SOURCE,
    "windows-command": SELECT_GUARD + WINDOWS_SETUP + "\n" + TEST_RUN,
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


def commands(os_name, fail="", name="command", source=None, tests="all", test_filter="all()"):
    validate_source(WORKFLOW.read_text() if source is None else source, name)
    return subprocess.run(
        ["bash", "-euo", "pipefail", "-c", HARNESS],
        env={**os.environ, "RUNNER_OS": os_name, "FAIL": fail, "BLOCK": name,
             "CRAFT_TESTS": tests, "CRAFT_TEST_FILTER": test_filter},
        text=True, capture_output=True, check=False,
    )


COMMON = [
    "fmt --check",
    "clippy --workspace --all-targets --locked -- -D warnings",
    "clippy --workspace --all-targets --all-features --locked -- -D warnings",
]
def test_commands(test_filter="all()"):
    return [line.strip()[len("cargo "):]
            .replace(DEFAULT_FILTER_ARGUMENT, f"({test_filter}) & ({DEFAULT_FEATURE_TESTS})")
            .replace('"$CRAFT_TEST_FILTER"', test_filter)
            for line in TEST_RUN.splitlines() if line.strip()]


TESTS = test_commands()


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

    def test_a_selection_filters_every_nextest_run_and_none_runs_nothing(self):
        for selection, selected in (("all", "all()"),
                                    ("some", "package(=craft-image-codecs) | package(=craft-text)"),
                                    ("some", "package(=craft-text)"),
                                    ("unknown", "package(=craft-text) | test(/ordinary/)")):
            with self.subTest(selection=selection, selected=selected):
                result = commands("Linux", tests=selection, test_filter=selected)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.splitlines(), test_commands(selected))
                # Doctests stay whole-workspace; only the last leg adds the default selector.
                self.assertIn("test --workspace --doc --all-features --locked", result.stdout.splitlines())
                for line in result.stdout.splitlines():
                    if line.startswith("nextest run -p craft-image-codecs -p craft-text"):
                        self.assertIn(f"-E ({selected}) & ({DEFAULT_FEATURE_TESTS}) --no-tests=warn", line)
                    elif line.startswith("nextest"):
                        self.assertIn(f"-E {selected} --no-tests=warn", line)
        result = commands("Linux", tests="none", test_filter="none()")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), ["No crate changed: no tests to run."])

    def test_default_name_matchers_keep_legacy_and_prefixed_checks(self):
        # These exercise the actual test-name regexes, not a second Nextest filter parser.
        # Package/binary predicates and the complete filter are checked by the literal shell contract.
        patterns = re.findall(r"test\(/([^/]+)/\)", DEFAULT_FEATURE_TESTS)
        for name in ("codecs::heif::tests::without_the_feature_heif_is_a_clear_unsupported_error",
                     "limits::max_pixels_enforced_for_every_format", "malformed::truncated_at_many_offsets",
                     "roundtrip::one_pixel_images_roundtrip", "hyphen::tests::language_names_and_codes",
                     "segment::tests::thai_without_its_feature_breaks_as_uax14",
                     "jpeg_xl_is_export_only_without_the_jxl_feature_and_detects_both_signatures",
                     "feature_config_new_default_check", "nested::feature_config_new_default_check"):
            with self.subTest(name=name):
                self.assertTrue(any(re.search(pattern, name) for pattern in patterns))
        for name in ("roundtrip::png_rgba_u16", "ordinary_roundtrip",
                     "detect_caps::ftyp_brands_respect_prefix_and_declared_bounds",
                     "detect_caps::detect_ftyp_keeps_its_short_brand_scan",
                     "detect_caps::detect_magic_png", "malformed::random_bytes_never_panic",
                     "hyphen::tests::bundled_data_loads",
                     "feature_config_group::nested::ordinary", "nested::not_feature_config_check"):
            with self.subTest(name=name):
                self.assertFalse(any(re.search(pattern, name) for pattern in patterns))

    def test_default_selector_cannot_escape_the_incoming_selection(self):
        source = WORKFLOW.read_text()
        original = "-E " + DEFAULT_FILTER_ARGUMENT
        for replacement in ('-E "$CRAFT_TEST_FILTER"',
                            f'-E "({DEFAULT_FEATURE_TESTS})"',
                            f'-E "$CRAFT_TEST_FILTER & ({DEFAULT_FEATURE_TESTS})"',
                            f'-E "($CRAFT_TEST_FILTER) & {DEFAULT_FEATURE_TESTS}"',
                            '-E "($CRAFT_TEST_FILTER) & test(/(^|::)feature_config_[^:]*$/)"'):
            with self.subTest(replacement=replacement), patch("subprocess.run") as child:
                with self.assertRaisesRegex(AssertionError, "unexpected command source"):
                    commands("Linux", source=source.replace(original, replacement, 1))
                child.assert_not_called()

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
        self.assertEqual(cargo, [line.strip() for line in TEST_RUN.splitlines() if line.strip()])

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
        # The macOS and Windows legs run the Linux leg's blocks, not copies of them.
        self.assertIn("      lint-command: &lint |\n", source)
        self.assertIn("      command: &command |\n", source)
        self.assertIn("      lint-command: *lint\n", source)
        self.assertIn("      command: *command\n", source)
        # Only the Linux leg keeps workspace crates; the cache budget has no room for the others.
        self.assertEqual(source.count("cache-workspace-crates: true"), 1)
        self.assertIn("  msrv:", source)
        self.assertIn("  wasm:", source)
        self.assertIn("python3 scripts/test-workspace-tests.py", block(source, "lint-command"))


if __name__ == "__main__":
    unittest.main()

