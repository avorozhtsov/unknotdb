from __future__ import annotations

import sys
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))

import audit_brittenham_b5 as audit


class BrittenhamB5AuditTests(unittest.TestCase):
    def test_exact_determinant_known_braid_closures(self) -> None:
        # Successive Markov stabilizations of the unknot.
        self.assertEqual(audit.knot_determinant([1, 2, 3, 4]), 1)
        # Successive stabilizations of the right-handed trefoil.
        self.assertEqual(audit.knot_determinant([1, 1, 1, 2, 3, 4]), 3)

    def test_published_two_change_successor_is_not_unknot(self) -> None:
        word = audit.SOURCE_WORD[:]
        for position in audit.FIXED_POSITIONS:
            word[position] = -word[position]
        self.assertEqual(audit.knot_determinant(word), 261)


if __name__ == "__main__":
    unittest.main()
