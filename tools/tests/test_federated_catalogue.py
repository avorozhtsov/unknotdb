from __future__ import annotations

import csv
import sqlite3
import sys
import tempfile
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))

import build_federated_catalogue as catalogue
import import_brittenham_adjacency as brittenham
import unknotdb_cli as cli


class FederatedCatalogueTests(unittest.TestCase):
    def setUp(self) -> None:
        self.db = sqlite3.connect(":memory:")
        catalogue.create_schema(self.db)
        self.db.execute(
            "INSERT INTO catalogue_sources VALUES (?,?,?,?,?,?,?)",
            (
                "fixture",
                "test",
                "https://example.invalid/fixture",
                "2026-08-27",
                "0" * 64,
                "TSV",
                "test fixture",
            ),
        )
        self.db.executemany(
            "INSERT INTO knots VALUES (?,?,?,?)",
            ((1, "3_1", 3, "rolfsen"), (2, "0_1", 0, "unknot")),
        )
        self.db.executemany(
            "INSERT INTO knot_identifiers VALUES (?,?,?,?,?,?)",
            (
                ("knotinfo", "3_1", 1, "canonical", "fixture", "fixture"),
                ("knotinfo", "0_1", 2, "canonical", "fixture", "fixture"),
            ),
        )

    def tearDown(self) -> None:
        self.db.close()

    def test_identifier_resolution(self) -> None:
        self.assertEqual(cli.resolve_knot(self.db, "3_1", None), 1)
        self.assertEqual(cli.resolve_knot(self.db, "3_1", "knotinfo"), 1)
        with self.assertRaisesRegex(ValueError, "unknown"):
            cli.resolve_knot(self.db, "9_99", None)

    def test_pd_graph_join_is_bidirectional_and_conservative(self) -> None:
        pd = "[[1,4,2,3]]"
        digest = catalogue.text_sha256("pd", pd)
        self.db.execute(
            "INSERT INTO representations VALUES (?,?,?,?,?,?,?,?)",
            (1, 1, "pd", pd, digest, "fixture", "row:1", 0),
        )
        identifications = sqlite3.connect(":memory:")
        identifications.executescript(
            """
            CREATE TABLE graph_vertices(
                rep_key BLOB PRIMARY KEY,node_id INTEGER,encoding BLOB,
                u_upper INTEGER,cc0_component_key BLOB);
            CREATE TABLE graph_vertex_knot_map(
                rep_key BLOB PRIMARY KEY,knot_id TEXT,mirror_bit INTEGER,
                evidence_class TEXT,mapping_status TEXT,evidence_id BLOB);
            CREATE TABLE knot_representation_postings(
                knot_id TEXT,representation_namespace TEXT,
                representation_ref TEXT,rep_key BLOB,graph_node_id INTEGER,
                mirror_bit INTEGER,evidence_class TEXT,mapping_status TEXT);
            """
        )
        key = bytes.fromhex("12" * 32)
        identifications.execute(
            "INSERT INTO graph_vertices VALUES (?,?,?,?,?)",
            (key, 7, b"braid", 1, key),
        )
        identifications.execute(
            "INSERT INTO graph_vertex_knot_map VALUES (?,?,?,?,?,?)",
            (key, "knot:3_1", 0, "verified", "fixture", b"evidence"),
        )
        identifications.execute(
            "INSERT INTO knot_representation_postings VALUES (?,?,?,?,?,?,?,?)",
            ("knot:3_1", "graph", key.hex(), key, 7, 0, "verified", "fixture"),
        )

        forward = cli.pd_rows_for_graph(
            identifications, self.db, key.hex(), limit=10
        )
        reverse = cli.graph_rows_for_pd(
            identifications, self.db, "[ [1, 4, 2, 3] ]", None, limit=10
        )
        self.assertEqual(forward[0]["pd"], pd)
        self.assertEqual(reverse[0]["graph_node_id"], 7)
        self.assertEqual(forward[0]["relation"], "same_knot_type")
        self.assertFalse(forward[0]["exact_conversion"])
        self.assertFalse(reverse[0]["exact_conversion"])
        identifications.close()

    def test_diagram_attested_adjacency_import(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "adjacency.tsv"
            fields = (
                "source_id",
                "source_url",
                "retrieved_at",
                "source_sha256",
                "source_format",
                "source_scope",
                "source_knot",
                "target_knot",
                "status",
                "claim_scope",
                "source_representation_encoding",
                "source_representation",
                "crossing_locator",
                "source_pointer",
            )
            with path.open("w", encoding="utf-8", newline="") as stream:
                writer = csv.DictWriter(stream, delimiter="\t", fieldnames=fields)
                writer.writeheader()
                writer.writerow(
                    {
                        "source_id": "external-fixture",
                        "source_url": "https://example.invalid/adjacency",
                        "retrieved_at": "2026-08-27",
                        "source_sha256": "1" * 64,
                        "source_format": "TSV",
                        "source_scope": "one fixed diagram",
                        "source_knot": "3_1",
                        "target_knot": "0_1",
                        "status": "diagram_attested",
                        "claim_scope": "fixed_diagram",
                        "source_representation_encoding": "pd",
                        "source_representation": "[[1,5,2,4]]",
                        "crossing_locator": "pd-crossing:0",
                        "source_pointer": "row:1",
                    }
                )
            inserted = catalogue.import_adjacency_tsv(
                self.db, [path], {"3_1": 1, "0_1": 2}
            )
        self.assertEqual(inserted, 1)
        self.assertEqual(
            self.db.execute(
                "SELECT status,claim_scope,crossing_locator FROM adjacency_claims"
            ).fetchone(),
            ("diagram_attested", "fixed_diagram", "pd-crossing:0"),
        )
        self.assertEqual(self.db.execute("PRAGMA foreign_key_check").fetchall(), [])

    def test_brittenham_distance_is_mirror_normalized(self) -> None:
        self.assertEqual(brittenham.distance_and_position([4, -6, 2], 3), (1, 1, False))
        self.assertEqual(brittenham.distance_and_position([-4, 6, -2], 3), (1, 1, True))
        self.assertEqual(
            brittenham.distance_and_position([4, 6, 2], 3), (0, None, False)
        )

    def test_brittenham_prime_line_parser(self) -> None:
        target, rest = brittenham.split_prime_line(
            "{ 3 4 6 2} 12 1 4 8 10 14 -2 16 20 6 22 12 24 18"
        )
        self.assertEqual(target, (3, 4, 6, 2))
        self.assertEqual(rest[:3], [12, 1, 4])
        self.assertEqual(len(rest[2:]), 12)


if __name__ == "__main__":
    unittest.main()
