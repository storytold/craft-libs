#!/usr/bin/env python3
"""Check the Workspace workflow's cancellation policy: pull requests and pushes to main cancel
the run they supersede; release tags, manual runs and scheduled runs are never cancelled.

The policy is the top-level `concurrency` block of .github/workflows/workspace.yml. This script
reads those fields, requires the documented expression shapes, and checks which group each event
would be given. The rest of the workflow is left to diff review, not frozen here.

Use --workflow PATH to read a baseline or a deliberately wrong variant without touching this
checkout.
"""

import argparse
import re
import sys
import unittest
from pathlib import Path

try:  # cross-check only; the reader below is what a plain `python3` run needs
    import yaml
except ImportError:
    yaml = None

DEFAULT_WORKFLOW = Path(__file__).resolve().parent.parent / ".github" / "workflows" / "workspace.yml"
PULL_REQUEST, PUSH, DISPATCH, SCHEDULE = "pull_request", "push", "workflow_dispatch", "schedule"

# The runs that share a group per ref and cancel the run they supersede. GitHub reads `a || b && c`
# as `a || (b && c)`, and `x && github.ref || github.run_id` as "github.ref, or the run id when x is
# false". This exact condition is the whole of the expression language this script understands, so
# a reworded policy has to be reworded here too, on purpose. github.ref, never github.head_ref or
# github.sha.
SUPERSEDED = (
    "github.event_name == 'pull_request'"
    " || github.event_name == 'push' && github.ref == 'refs/heads/main'"
)
GROUP_SHAPE = re.compile(
    r"(?P<prefix>[A-Za-z0-9._-]+)-\$\{\{\s*\(\s*(?P<condition>[^()]+?)\s*\)\s*"
    r"&&\s*github\.ref\s*\|\|\s*github\.run_id\s*\}\}"
)
CANCEL_SHAPE = re.compile(r"\$\{\{\s*(?P<condition>[^{}]+?)\s*\}\}")


def normal(expression):
    return re.sub(r"\s+", " ", expression).strip()


def superseded(event_name, ref):
    """SUPERSEDED, evaluated."""
    return event_name == PULL_REQUEST or (event_name == PUSH and ref == "refs/heads/main")


def read_concurrency(text):
    """The top-level `concurrency` mapping. A deliberately limited reader: two-space fields."""
    lines = text.splitlines()
    starts = [index for index, line in enumerate(lines) if line.startswith("concurrency:")]
    if len(starts) != 1:
        raise AssertionError(f"expected one top-level concurrency block, found {len(starts)}")
    header = lines[starts[0]].partition("#")[0].strip()
    if header != "concurrency:":
        # `concurrency: |` and `concurrency: some-string` are scalars, not a mapping. Reading the
        # lines under one as fields would let a workflow that is not this policy pass every check.
        raise AssertionError(f"concurrency must be a mapping, not a scalar: {lines[starts[0]]!r}")
    fields = {}
    for line in lines[starts[0] + 1:]:
        stripped = line.strip()
        if stripped and not line.startswith(" "):
            break  # the next top-level key
        if not stripped or stripped.startswith("#"):
            continue
        if not line.startswith("  ") or line.startswith("   "):
            raise AssertionError(f"unexpected indentation under concurrency: {line!r}")
        key, separator, value = stripped.partition(":")
        if not separator:
            raise AssertionError(f"not a field of concurrency: {line!r}")
        fields[key.strip()] = value.strip()
    return fields


def pull_request(number, run_id):
    # github.ref for a pull request is refs/pull/<n>/merge: the PR number, never the branch name.
    return (PULL_REQUEST, f"refs/pull/{number}/merge", run_id)


def other_run(event_name, ref, run_id):
    return (event_name, ref, run_id)


MAIN, TAG = ("refs/heads/main", "refs/tags/v1.2.3")
SAME_GROUP = [
    ("a newer commit on the same pull request", pull_request(1316, 43599251877), pull_request(1316, 43600899878)),
    ("two pushes to main", other_run(PUSH, MAIN, 3), other_run(PUSH, MAIN, 4)),
]
DIFFERENT_GROUPS = [
    ("two pull requests", pull_request(1316, 1), pull_request(1317, 1)),
    ("the same branch name in two forks", pull_request(1316, 2), pull_request(1400, 2)),
    ("two release tag runs", other_run(PUSH, TAG, 5), other_run(PUSH, TAG, 6)),
    ("two manual runs", other_run(DISPATCH, MAIN, 7), other_run(DISPATCH, MAIN, 8)),
    ("two scheduled runs", other_run(SCHEDULE, MAIN, 15), other_run(SCHEDULE, MAIN, 16)),
    ("a main push and a scheduled run", other_run(PUSH, MAIN, 17), other_run(SCHEDULE, MAIN, 17)),
    ("a main push and a manual run of main", other_run(PUSH, MAIN, 18), other_run(DISPATCH, MAIN, 18)),
    ("a main push and a pull request", other_run(PUSH, MAIN, 9), pull_request(1316, 9)),
    ("a release tag and a pull request", other_run(PUSH, TAG, 10), pull_request(1316, 10)),
]
CANCELLATION = [
    ("a push supersedes the previous run of its pull request", pull_request(1316, 11), True),
    ("a push to main supersedes the previous main run", other_run(PUSH, MAIN, 12), True),
    ("a release tag run is never cancelled", other_run(PUSH, TAG, 13), False),
    ("a manual run is never cancelled", other_run(DISPATCH, MAIN, 14), False),
    ("the scheduled run that saves the caches is never cancelled", other_run(SCHEDULE, MAIN, 19), False),
]


class ConcurrencyTests(unittest.TestCase):
    workflow = DEFAULT_WORKFLOW

    @classmethod
    def setUpClass(cls):
        cls.text = cls.workflow.read_text()
        cls.fields = read_concurrency(cls.text)
        cls.group_shape = GROUP_SHAPE.fullmatch(cls.fields.get("group", ""))
        cls.cancel_shape = CANCEL_SHAPE.fullmatch(cls.fields.get("cancel-in-progress", ""))
        if not cls.group_shape or not cls.cancel_shape:
            raise AssertionError(f"the concurrency fields are not the documented shapes: {cls.fields}")

    def cancels(self, event_name, ref):
        return superseded(event_name, ref)

    def group(self, event_name, ref, run_id):
        return self.group_shape["prefix"] + "-" + (ref if superseded(event_name, ref) else str(run_id))

    def test_policy_is_the_two_documented_fields(self):
        self.assertEqual(sorted(self.fields), ["cancel-in-progress", "group"])
        self.assertEqual(self.group_shape["prefix"], "workspace")
        self.assertEqual(normal(self.group_shape["condition"]), SUPERSEDED)
        self.assertEqual(normal(self.cancel_shape["condition"]), SUPERSEDED)

    def test_a_scalar_concurrency_value_is_not_read_as_a_mapping(self):
        cases = {
            "a block scalar": "concurrency: |\n  group: workspace-x\n  cancel-in-progress: true\n",
            "a plain scalar": "concurrency: workspace-x\njobs: {}\n",
        }
        for description, text in cases.items():
            with self.subTest(description):
                with self.assertRaises(AssertionError):
                    read_concurrency(text)

    def test_one_group_per_pull_request_and_run(self):
        for description, first, second in SAME_GROUP:
            with self.subTest(description):
                self.assertEqual(self.group(*first), self.group(*second), description)
        for description, first, second in DIFFERENT_GROUPS:
            with self.subTest(description):
                self.assertNotEqual(self.group(*first), self.group(*second), description)

    def test_only_pull_requests_and_main_pushes_cancel(self):
        for description, context, expected in CANCELLATION:
            with self.subTest(description):
                self.assertEqual(self.cancels(context[0], context[1]), expected, description)

    def test_pyyaml_reads_the_same_fields(self):
        if yaml is None:
            self.skipTest("PyYAML is not installed; the limited reader above ran alone")
        document = yaml.load(self.text, Loader=yaml.BaseLoader)  # BaseLoader keeps `on:` a string
        self.assertEqual(document["concurrency"], self.fields)

    def test_no_per_job_concurrency_is_added(self):
        indented = [line for line in self.text.splitlines()
                    if line.strip().startswith("concurrency:") and not line.startswith("concurrency:")]
        self.assertEqual(indented, [], "the policy belongs to the workflow, not to one job")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--workflow", type=Path, default=DEFAULT_WORKFLOW, help="workflow to read (default: %(default)s)")
    arguments, remaining = parser.parse_known_args()
    ConcurrencyTests.workflow = arguments.workflow
    unittest.main(argv=[sys.argv[0], *remaining])


if __name__ == "__main__":
    main()
