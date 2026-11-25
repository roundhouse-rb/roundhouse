"""The row join in lobsters-specs: Rails' own RSpec run against the emit's.

A spec Rails passes that the emit never produced is `dropped` (the file
failed to load, so there is no example to fail); one that fails on Rails
too is `env` and must not be charged to a cause. The join is a pure
function of the two reports (scripts/lib/lobsters_join.rb), so it is
exercised here without an emit tree, the shim, a bundle or a Rails boot.
"""

from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
JOIN = ROOT / "scripts/lib/lobsters_join.rb"
SHA = "28bc8d6a1b2c3d4e5f60718293a4b5c6d7e8f901"
BASE = "spec/models/comment_spec.rb"

# One example per status. The emit run is handed absolute suite paths and
# the Rails run reports `./spec/...`, so the ids differ by prefix only.
EMIT_EXAMPLES = [
    {"id": f"/tmp/lobsters/{BASE}[1:1:1]", "status": "passed"},
    {"id": f"/tmp/lobsters/{BASE}[1:1:2]", "status": "failed"},
    {"id": f"/tmp/lobsters/{BASE}[1:1:3]", "status": "pending"},
    {"id": f"/tmp/lobsters/{BASE}[1:1:5]", "status": "failed"},
]
RAILS_TESTS = [
    {"id": f"./{BASE}[1:1:1]", "name": "passes on both", "file": BASE, "status": "pass"},
    {"id": f"./{BASE}[1:1:2]", "name": "fails on the emit", "file": BASE, "status": "pass"},
    # Rails runs it, the emit produced no example with that id.
    {
        "id": f"./{BASE}[1:1:4]",
        "name": "the emit never generated it",
        "file": BASE,
        "status": "pass",
    },
    # Fails on stock Rails: environment, not the compiler.
    {
        "id": f"./{BASE}[1:1:5]",
        "name": "order dependent",
        "file": BASE,
        "status": "fail",
        "exception": "RSpec::Expectations::ExpectationNotMetError",
        "message": "expected 1, got 0",
    },
    {"id": f"./{BASE}[1:1:3]", "name": "skipped", "file": BASE, "status": "skip"},
]


class LobstersJoinTests(unittest.TestCase):
    def join(self, emit_examples=EMIT_EXAMPLES, rails_tests=RAILS_TESTS) -> dict:
        with tempfile.TemporaryDirectory(prefix="lobsters-join-") as tmp:
            emit = Path(tmp) / "rspec.json"
            baseline = Path(tmp) / "rails-baseline.json"
            emit.write_text(json.dumps({"examples": emit_examples}))
            baseline.write_text(json.dumps({"sha": SHA, "tests": rails_tests}))
            result = subprocess.run(
                [
                    "ruby",
                    "-r", str(JOIN),
                    "-r", "json",
                    "-e",
                    "puts JSON.generate(LobstersJoin.rows("
                    "emit_examples: JSON.parse(File.read(ARGV[0]))['examples'], "
                    "baseline: JSON.parse(File.read(ARGV[1]))))",
                    str(emit),
                    str(baseline),
                ],
                check=False, text=True, capture_output=True,
            )
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        return json.loads(result.stdout)

    def test_rows_classify_match_gap_dropped_env_and_skip(self):
        joined = self.join()
        self.assertEqual(
            joined["rows"],
            {"match": 1, "gap": 1, "dropped": 1, "env": 1, "skip": 1},
        )
        # Rows are over the examples RAILS runs, so they sum to that
        # count — the emit's own extra examples are not rows.
        self.assertEqual(sum(joined["rows"].values()), len(RAILS_TESTS))
        self.assertEqual(
            joined["dropped"],
            [{"id": f"{BASE}[1:1:4]", "name": "the emit never generated it", "file": BASE}],
        )
        self.assertEqual(
            joined["env"],
            [
                {
                    "id": f"{BASE}[1:1:5]",
                    "name": "order dependent",
                    "file": BASE,
                    "exception": "RSpec::Expectations::ExpectationNotMetError",
                    "message": "expected 1, got 0",
                }
            ],
        )
        # The caller keeps these ids out of its cause counts: the env
        # example fails on the emit too, and it is Rails' failure, not
        # the compiler's.
        self.assertEqual(joined["env_ids"], {f"{BASE}[1:1:5]": True})

    def test_normalizes_dot_and_absolute_prefixes_to_one_id(self):
        ids = {
            "spec/models/x_spec.rb[1:2:1]",
            "./spec/models/x_spec.rb[1:2:1]",
            f"/tmp/lobsters/spec/models/x_spec.rb[1:2:1]",
        }
        result = subprocess.run(
            ["ruby", "-r", str(JOIN), "-e",
             "puts ARGV.map { |a| LobstersJoin.example_id(a) }", *sorted(ids)],
            check=False, text=True, capture_output=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.split(), ["spec/models/x_spec.rb[1:2:1]"] * 3)

    def test_an_empty_baseline_joins_nothing_rather_than_everything(self):
        # Rows are over the examples RAILS runs, so a run that recorded
        # none makes no claim about the emit's examples at all — it does
        # not turn them into a wall of `dropped`.
        joined = self.join(rails_tests=[])
        self.assertEqual(
            joined["rows"],
            {"match": 0, "gap": 0, "dropped": 0, "env": 0, "skip": 0},
        )
        self.assertEqual(joined["dropped"], [])


if __name__ == "__main__":
    unittest.main()
