"""The row join in app-suite-summary: Rails' own run against the emit's.

A test Rails passes that the emit never generated is `dropped`; one that
fails on Rails too is `env` and must not be charged to a cause. The join is
a pure function of a tally, a fail-log, the emitted test list and the
baseline, so it is exercised here without a transpile.
"""

from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SUMMARY = ROOT / "scripts/app-suite-summary"
REPORT = ROOT / "scripts/app-suite-report"
SHA = "abc123def4567890"


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


class AppSuiteJoinTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="app-suite-join-")
        self.addCleanup(self.tmp.cleanup)
        self.dir = Path(self.tmp.name)
        # Two emitted tests, one passing and one failing with an origin.
        write(self.dir / "tally.txt", "FAIL|test/alpha_test|1|2|boom\n")
        write(
            self.dir / "fail.txt",
            "test/alpha_test|AlphaTest#test_broken|boom|RuntimeError|app/x.rb:1:in 'x'\n",
        )
        write(
            self.dir / "emit.txt",
            "AlphaTest#test_ok|pass\nAlphaTest#test_broken|fail\n",
        )
        write(
            self.dir / "baseline.json",
            json.dumps(
                {
                    "app": "fixture",
                    "repo": "example/fixture",
                    "sha": SHA,
                    "rails": "8.2.0.alpha",
                    "ruby": {"used": "4.0.9", "pinned": "4.0.7"},
                    "generated_at": "2026-11-16T13:17:53Z",
                    "tests": [
                        {"name": "AlphaTest#test_ok", "file": "test/alpha_test.rb", "status": "pass"},
                        {"name": "AlphaTest#test_broken", "file": "test/alpha_test.rb", "status": "pass"},
                        # Rails has it; the emit has no such test at all.
                        {
                            "name": "MiddlewareTest#test_guard",
                            "file": "test/middleware/middleware_test.rb",
                            "status": "pass",
                        },
                        # Fails on stock Rails: environment, not the compiler.
                        {
                            "name": "VipsTest#test_libheif",
                            "file": "test/models/vips_test.rb",
                            "status": "fail",
                            "exception": "LoadError",
                            "message": "libheif missing",
                        },
                    ],
                }
            ),
        )

    def _join(self, sha: str = SHA) -> tuple[dict, str]:
        out = self.dir / "summary.json"
        result = subprocess.run(
            [
                "ruby", str(SUMMARY),
                "--tally", str(self.dir / "tally.txt"),
                "--fail-log", str(self.dir / "fail.txt"),
                "--emit-tests", str(self.dir / "emit.txt"),
                "--baseline", str(self.dir / "baseline.json"),
                "--app", "fixture",
                "--app-sha", sha,
                "--out", str(out),
            ],
            check=False, text=True, capture_output=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        return json.loads(out.read_text()), result.stderr

    def test_rows_classify_match_gap_dropped_and_env(self):
        summary, _ = self._join()
        self.assertEqual(
            summary["rows"],
            {"match": 1, "gap": 1, "dropped": 1, "env": 1, "skip": 0, "extra": 0},
        )
        self.assertEqual(
            summary["dropped"],
            [{"name": "MiddlewareTest#test_guard", "file": "test/middleware/middleware_test.rb"}],
        )
        self.assertEqual(
            summary["env"],
            [{"name": "VipsTest#test_libheif", "exception": "LoadError", "message": "libheif missing"}],
        )
        self.assertEqual(summary["extra"], [])
        # The env row is set aside: the tally's one failure is a cause, and
        # the env test is not a second one.
        self.assertEqual(summary["tests"]["failed"], 1)
        causes = {c["slug"]: c for c in summary["causes"]}
        self.assertEqual(causes["dropped:test/middleware"]["owner"], "compiler")
        self.assertEqual(causes["dropped:test/middleware"]["failures"], 1)
        self.assertIn("never generated", causes["dropped:test/middleware"]["title"])
        self.assertEqual(causes["unclassified"]["failures"], 1)
        self.assertEqual(causes["unclassified"]["examples"][0]["exception"], "RuntimeError")
        self.assertEqual(
            causes["unclassified"]["examples"][0]["frame"], "app/x.rb:1:in 'x'"
        )

    def test_report_names_the_app_and_shows_the_dropped_section(self):
        self._join()
        out = self.dir / "index.html"
        result = subprocess.run(
            ["ruby", str(REPORT), str(self.dir / "summary.json"), str(out)],
            check=False, text=True, capture_output=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        html = out.read_text()
        self.assertIn("<title>Fixture conformance", html)
        self.assertIn("Tests Rails runs that the emit never ran", html)
        self.assertIn("Set aside: fails on Rails too", html)
        self.assertIn("of 4 tests Rails runs", html)

    def test_env_failures_the_emit_also_fails_are_set_aside_but_compiler_ones_stay(self):
        # One file, three tests failing on the emit: an env test with its own
        # FAIL line, and an env test and a compiler failure with none (the
        # shortfall). A fourth env test passes on the emit and must not
        # shrink the shortfall: only the compiler failure is charged.
        write(self.dir / "tally.txt", "FAIL|test/beta_test|1|4|boom\n")
        write(
            self.dir / "fail.txt",
            "test/beta_test|BetaTest#test_env_listed|heif|LoadError|app/v.rb:1:in 'v'\n",
        )
        write(
            self.dir / "emit.txt",
            "BetaTest#test_env_listed|fail|test/beta_test\n"
            "BetaTest#test_env_unlisted|fail|test/beta_test\n"
            "BetaTest#test_ours|fail|test/beta_test\n"
            "BetaTest#test_env_passing|pass|test/beta_test\n",
        )
        baseline = json.loads((self.dir / "baseline.json").read_text())
        baseline["tests"] = [
            {"name": f"BetaTest#{t}", "file": "test/beta_test.rb", "status": s}
            for t, s in [
                ("test_env_listed", "fail"),
                ("test_env_unlisted", "fail"),
                ("test_ours", "pass"),
                ("test_env_passing", "fail"),
            ]
        ]
        write(self.dir / "baseline.json", json.dumps(baseline))
        summary, _ = self._join()
        self.assertEqual(summary["rows"]["env"], 3)
        self.assertEqual(summary["rows"]["gap"], 1)
        self.assertEqual(summary["tests"]["failed"], 3)
        charged = {c["slug"]: c["failures"] for c in summary["causes"] if c["failures"]}
        self.assertEqual(charged, {"unclassified": 1})

    def test_sha_mismatch_refuses_to_join(self):
        summary, stderr = self._join(sha="deadbeef")
        self.assertIsNone(summary["rows"])
        self.assertIn("does not match this run", stderr)
        self.assertIn("Not joined", summary["baseline"]["note"])
        self.assertEqual(summary["dropped"], [])


if __name__ == "__main__":
    unittest.main()
