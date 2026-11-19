"""Regression tests for the optional black-box checker; no real raw files required."""

import contextlib
import hashlib
import io
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import verify_orf_reference as reference


INPUT = b"fixture"
PIXELS = bytes.fromhex("1234abcd")
INPUT_HASH = hashlib.sha256(INPUT).hexdigest()
SENSOR_HASH = hashlib.sha256(bytes.fromhex("3412cdab")).hexdigest()
ROW = f'("sample.orf", "{INPUT_HASH}", (2, 1), "{SENSOR_HASH}"),'


def manifest(rows):
    return f'const CASES: &[(&str, &str, (usize, usize), &str)] = &[\n{rows}\n];\n'


class ManifestTests(unittest.TestCase):
    def test_real_manifest_has_all_23_references(self):
        cases = reference.parse_cases(reference.CASES)
        self.assertEqual(len(cases), 23)
        self.assertEqual((cases[0][0], cases[0][2:4]), ("orf-olympus-epl7.orf", (4640, 3472)))
        self.assertEqual((cases[-1][0], cases[-1][2:4]), ("orf-tg-7-6946.orf", (4040, 3016)))

    def test_invalid_manifest_cannot_silently_drop_a_reference(self):
        invalid = ["", manifest(""), manifest(ROW + "\n(\"incomplete.orf\","),
                   manifest(ROW.replace(INPUT_HASH, "bad")), manifest(ROW + "\nunknown"),
                   manifest(ROW.replace("(2, 1)", "(0, 1)")), manifest(ROW + "\n" + ROW)]
        with tempfile.TemporaryDirectory(prefix="craft-orf-test-") as directory:
            path = pathlib.Path(directory) / "cases.rs"
            for text in invalid:
                with self.subTest(text=text):
                    path.write_text(text, encoding="utf-8")
                    with self.assertRaises(ValueError):
                        reference.parse_cases(path)

    def test_repeated_filename_outside_the_array_is_not_a_fixture(self):
        with tempfile.TemporaryDirectory(prefix="craft-orf-test-") as directory:
            path = pathlib.Path(directory) / "cases.rs"
            path.write_text(manifest(ROW) + 'fn example() { assert_eq!(name, "sample.orf"); }', encoding="utf-8")
            self.assertEqual(reference.parse_cases(path), [("sample.orf", INPUT_HASH, 2, 1, SENSOR_HASH)])


class SensorHashTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="craft-orf-test-")
        self.addCleanup(directory.cleanup)
        self.path = pathlib.Path(directory.name) / "sensor.pgm"

    def test_comments_and_split_header_preserve_full_sensor_hash(self):
        self.path.write_bytes(b"P5\n# sensor\n2 # width\n1\n65535\n" + PIXELS)
        self.assertEqual(reference.sensor_hash(self.path, 2, 1), SENSOR_HASH)

    def test_sensor_larger_than_one_read_block(self):
        width = 524290
        self.path.write_bytes(f"P5\n{width} 1\n65535\n".encode() + b"\x12\x34" * width)
        expected = hashlib.sha256(b"\x34\x12" * width).hexdigest()
        self.assertEqual(reference.sensor_hash(self.path, width, 1), expected)

    def test_invalid_header_dimensions_and_payload_are_rejected(self):
        header = b"P5\n2 1\n65535\n"
        invalid = [
            (b"P2\n2 1\n65535\n" + PIXELS, "expected binary PGM"),
            (b"P5\n3 1\n65535\n" + PIXELS, "dimensions or scaling"),
            (b"P5\n2 1\n255\n" + PIXELS, "dimensions or scaling"),
            (b"P5\n2 1 65535 1\n" + PIXELS, "dimensions or scaling"),
            (b"P5\n" + b"#" * 4096 + b"\n", "invalid PGM header"),
            (b"P5\n# incomplete\n", "invalid PGM header"),
            (header + PIXELS[:2], "truncated sensor"),
            (header + PIXELS[:3], "truncated sensor"),
            (header + PIXELS + b"\0", "trailing sensor bytes"),
        ]
        for data, message in invalid:
            with self.subTest(message=message, data_length=len(data)):
                self.path.write_bytes(data)
                with self.assertRaisesRegex(ValueError, message):
                    reference.sensor_hash(self.path, 2, 1)

    def test_nonpositive_dimensions_are_rejected(self):
        for width, height in [(0, 1), (1, 0), (-1, 1)]:
            with self.subTest(width=width, height=height), self.assertRaisesRegex(ValueError, "empty sensor"):
                reference.sensor_hash(self.path, width, height)


@unittest.skipUnless(os.name == "posix", "synthetic executable uses a POSIX shebang")
class ReferenceRunnerTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="craft-orf-test-")
        self.addCleanup(directory.cleanup)
        self.root = pathlib.Path(directory.name)
        raw = self.root / "corpus/raw"
        raw.mkdir(parents=True)
        self.source = raw / "sample.orf"
        self.source.write_bytes(INPUT)
        self.fixtures = self.root / "cases.rs"
        self.fixtures.write_text(manifest(ROW), encoding="utf-8")
        self.oracle = self.root / "oracle"
        self.oracle.write_text(
            f"#!{sys.executable}\n"
            "import pathlib, sys\n"
            "if len(sys.argv) > 1:\n"
            "    pathlib.Path(sys.argv[2] + '.pgm').write_bytes(\n"
            "        b'P5\\n2 1\\n65535\\n' + bytes.fromhex('1234abcd'))\n",
            encoding="utf-8",
        )
        self.oracle.chmod(0o755)

    def run_checker(self, oracle="./oracle"):
        with (
            contextlib.chdir(self.root),
            mock.patch.object(reference, "CASES", self.fixtures),
            mock.patch.object(tempfile, "tempdir", str(self.root)),
            contextlib.redirect_stdout(io.StringIO()) as output,
        ):
            reference.main(["--corpus", "corpus", "--oracle", oracle])
        return output.getvalue()

    def test_relative_oracle_runs_from_scratch_and_leaves_no_media(self):
        self.assertIn("All 1 reference sensors match", self.run_checker())
        self.assertEqual(self.source.read_bytes(), INPUT)
        self.assertEqual(list(self.root.rglob("*.pgm")), [])
        self.assertEqual(list(self.root.glob("craft-orf-reference-*")), [])

    def test_input_hash_mismatch_is_rejected(self):
        self.source.write_bytes(b"different input")
        with self.assertRaisesRegex(ValueError, "input SHA-256 differs"):
            self.run_checker()

    def test_sensor_hash_mismatch_is_rejected_and_cleaned_up(self):
        self.fixtures.write_text(manifest(ROW.replace(SENSOR_HASH, "0" * 64)), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "sensor SHA-256 .* differs"):
            self.run_checker()
        self.assertEqual(list(self.root.glob("craft-orf-reference-*")), [])
        self.assertEqual(self.source.read_bytes(), INPUT)

    def test_oracle_failure_is_not_reported_as_a_match(self):
        self.oracle.write_text(f"#!{sys.executable}\nimport sys\nif len(sys.argv) > 1: sys.exit(7)\n", encoding="utf-8")
        with self.assertRaises(subprocess.CalledProcessError) as error:
            self.run_checker()
        self.assertEqual(error.exception.returncode, 7)
        self.assertEqual(list(self.root.glob("craft-orf-reference-*")), [])

    def test_missing_oracle_is_an_argument_error(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
            self.run_checker("./missing-oracle")
        self.assertEqual(error.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
