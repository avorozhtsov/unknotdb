from __future__ import annotations

import sys
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))

import attest_named_proof_chain as named_chain
import build_graph_identification_sidecar as graph_ids
import unknotdb_cli


class GraphIdentificationTests(unittest.TestCase):
    def test_packed_nibble_representation_decode(self) -> None:
        # B3 [1,-2,2,-1] in packed storage codec v1.
        self.assertEqual(
            graph_ids.decode_representation(bytes.fromhex("b10203043012")),
            (3, False, (1, -2, 2, -1)),
        )

    def test_legacy_representation_decode(self) -> None:
        encoded = (
            b"UKB0"
            + bytes((0, 0))
            + (2).to_bytes(2, "little")
            + (3).to_bytes(4, "little")
            + b"\x01\x00\xff\xff\x01\x00"
        )
        self.assertEqual(
            graph_ids.decode_representation(encoded),
            (2, False, (1, -1, 1)),
        )

    def test_cc0_disjoint_set_is_undirected_and_transitive(self) -> None:
        components = graph_ids.DisjointSet(5)
        components.union(0, 1)
        components.union(2, 1)
        components.union(3, 4)
        self.assertEqual(components.find(0), components.find(2))
        self.assertNotEqual(components.find(0), components.find(3))

    def test_named_chain_mask_flips_only_marked_crossings(self) -> None:
        cube = {"base_word": [1, -2, 3], "marked_positions": [0, 2]}
        self.assertEqual(named_chain.state_word(cube, 0), [1, -2, 3])
        self.assertEqual(named_chain.state_word(cube, 1), [-1, -2, 3])
        self.assertEqual(named_chain.state_word(cube, 3), [-1, -2, -3])

    def test_snappy_knot_names_normalize_to_sidecar_ids(self) -> None:
        self.assertEqual(unknotdb_cli.canonical_knot_id("K14a18636"), "knot:14a_18636")
        self.assertEqual(unknotdb_cli.canonical_knot_id("15n81556"), "knot:15n_81556")
        self.assertEqual(unknotdb_cli.canonical_knot_id("12n_412"), "knot:12n_412")


if __name__ == "__main__":
    unittest.main()
