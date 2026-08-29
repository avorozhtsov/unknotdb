from __future__ import annotations

import sys
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))

import build_dgkt_three_change_witnesses as dgkt


class DgktWitnessTests(unittest.TestCase):
    def test_crossing_change_is_an_involution(self) -> None:
        crossing = [1, 4, 2, 3]
        changed = dgkt.change_crossing(crossing, 4)
        self.assertEqual(dgkt.change_crossing(changed, 4), crossing)

    def test_tietze_eliminates_a_free_generator_relation(self) -> None:
        reduction = dgkt.tietze_reduce(
            {"generators": [1, 2], "relators": [[-2, 1]]}
        )
        self.assertTrue(reduction["reduces_to_infinite_cyclic"])
        self.assertEqual(reduction["remaining_generators"], [2])
        self.assertEqual(reduction["remaining_relators"], [])


if __name__ == "__main__":
    unittest.main()
