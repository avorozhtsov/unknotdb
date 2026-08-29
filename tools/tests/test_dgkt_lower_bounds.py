from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

MODULE_PATH = Path(__file__).parents[1] / "build_dgkt_lower_bounds.py"
SPEC = importlib.util.spec_from_file_location("build_dgkt_lower_bounds", MODULE_PATH)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class DgktLowerBoundTests(unittest.TestCase):
    def test_canonical_knot_ids(self) -> None:
        self.assertEqual(MODULE.canonical_knot_id("11n3"), "knot:11n_3")
        self.assertEqual(MODULE.canonical_knot_id("13a_1232"), "knot:13a_1232")
        self.assertEqual(MODULE.canonical_knot_id("9_19"), "knot:9_19")

    def test_interval_parser(self) -> None:
        self.assertEqual(MODULE.parse_interval("[2,4]"), (2, 4))
        with self.assertRaises(ValueError):
            MODULE.parse_interval("[4,2]")
        with self.assertRaises(ValueError):
            MODULE.parse_interval("2")

    def test_corrected_method_files_only(self) -> None:
        self.assertEqual(
            MODULE.SOURCE_METHODS["D&D"][1],
            "lower_bounds/owens/valid_determinant_group_certificates.csv",
        )
        self.assertNotIn("owens_u2", repr(MODULE.SOURCE_METHODS))


if __name__ == "__main__":
    unittest.main()
