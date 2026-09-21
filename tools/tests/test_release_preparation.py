from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from audit_release_candidate import compare_bounds


class ReleasePreparationTests(unittest.TestCase):
    def test_bounds_keep_unknown_and_regression_distinct(self):
        old = {b"a": 3, b"b": None, b"c": 2, b"d": 1, b"e": 0, b"f": None}
        new = {b"a": 2, b"b": 4, b"c": 3, b"d": None, b"f": None, b"g": 1}
        self.assertEqual(compare_bounds(old, new), {
            "common_canonical_keys": 5, "missing_old_canonical_keys": 1,
            "added_canonical_keys": 1, "newly_finite_upper_bounds": 1,
            "improved_finite_upper_bounds": 1, "regressed_upper_bounds": 2,
        })

    def test_identical_bounds_have_no_changes(self):
        rows = {b"a": None, b"b": 0}
        result = compare_bounds(rows, rows)
        self.assertEqual(result["common_canonical_keys"], 2)
        self.assertTrue(all(value == 0 for key, value in result.items()
                            if key != "common_canonical_keys"))
