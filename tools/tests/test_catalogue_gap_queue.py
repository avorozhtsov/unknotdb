from __future__ import annotations

import sys
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))

import build_catalogue_reidemeister_witnesses as witnesses
import build_release_catalogue_gap_queue as queue


class CatalogueGapQueueTests(unittest.TestCase):
    def test_catalogue_u_parser_is_fail_closed(self) -> None:
        self.assertEqual(queue.parse_u("2"), (2, 2, "exact"))
        self.assertEqual(queue.parse_u("[1, 3]"), (1, 3, "range"))
        self.assertIsNone(queue.parse_u("1 or 2"))
        self.assertIsNone(queue.parse_u("[3, 1]"))

    def test_knot_name_parser_accepts_rolfsen_and_htw_names(self) -> None:
        self.assertEqual(
            witnesses.KNOT_ID.fullmatch("knot:9_19").groups(), ("9", "", "19")
        )
        self.assertEqual(
            witnesses.KNOT_ID.fullmatch("knot:12a_864").groups(),
            ("12", "a", "864"),
        )
        self.assertEqual(
            witnesses.KNOT_ID.fullmatch("knot:12n_709").groups(),
            ("12", "n", "709"),
        )


if __name__ == "__main__":
    unittest.main()
