#!/usr/bin/env python3
"""Build compact representation, knot-ID and invariant lookup maps.

The result is a metadata sidecar. It is snapshot-pinned but not part of the
proof graph and cannot change replay-validated U upper bounds.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sqlite3
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def value_sha256(invariant_id: str, value_type: str, value: str) -> bytes:
    return hashlib.sha256(
        b"UNKNOTDB_INVARIANT_VALUE_V0\0"
        + invariant_id.encode()
        + b"\0"
        + value_type.encode()
        + b"\0"
        + value.encode()
    ).digest()


def canonical_json(value: Any) -> str:
    return json.dumps(value, separators=(",", ":"), sort_keys=True)


def load_representations(paths: list[Path]) -> dict[str, tuple[tuple[int, ...], int]]:
    result: dict[str, tuple[tuple[int, ...], int]] = {}
    for path in paths:
        document = json.loads(path.read_text())
        for entry in document["entries"]:
            identity = entry["representation_id"]
            candidate = (tuple(map(int, entry["word"])), int(entry["strands"]))
            previous = result.setdefault(identity, candidate)
            if previous != candidate:
                raise ValueError(f"conflicting braid encodings for {identity}")
    return result


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-sidecar", type=Path, required=True)
    parser.add_argument("--corpus-json", type=Path, action="append", required=True)
    parser.add_argument("--rf-src", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists() or args.report.exists():
        raise FileExistsError("output artifact already exists")

    representations = load_representations(args.corpus_json)
    source = sqlite3.connect(f"file:{args.source_sidecar}?mode=ro", uri=True)
    source_meta = dict(source.execute("SELECT key,value FROM meta"))
    graph_rows = list(
        source.execute(
            "SELECT representation_id,stopping_key,graph_node_id,graph_u_upper,status "
            "FROM representations ORDER BY representation_id"
        )
    )
    identification_rows = list(
        source.execute(
            """
            SELECT representation_id,canonical_name,count(*)
            FROM identifications
            GROUP BY representation_id,canonical_name
            ORDER BY representation_id,canonical_name
            """
        )
    )
    source_invariants = {
        name: {
            "crossing_number_catalogue": ("integer", str(crossings)),
            "determinant": ("integer", str(determinant_value)),
            "alexander": ("laurent-pairs-json", alexander_json),
            "jones": ("laurent-pairs-json", jones_json),
        }
        for name, crossings, determinant_value, alexander_json, jones_json in source.execute(
            "SELECT canonical_name,crossings,determinant,alexander_json,jones_json FROM invariants"
        )
    }
    source.close()

    by_representation: dict[str, tuple[str, int]] = {}
    by_knot: dict[str, list[str]] = defaultdict(list)
    for representation_id, canonical_name, evidence_count in identification_rows:
        if representation_id in by_representation:
            raise ValueError(f"representation has multiple knot IDs: {representation_id}")
        knot_id = f"knot:{canonical_name}"
        by_representation[representation_id] = (knot_id, int(evidence_count))
        by_knot[knot_id].append(representation_id)

    sys.path.insert(0, str(args.rf_src))
    from rf_knots.invariants import (
        alexander_polynomial,
        signature,
        to_pairs,
    )

    invariant_definitions = {
        "crossing_number_catalogue": (
            "integer",
            "knot",
            "Crossing number encoded by the canonical table identifier",
        ),
        "determinant": ("integer", "knot", "Absolute Alexander evaluation at -1"),
        "alexander": (
            "laurent-pairs-json",
            "knot",
            "Canonical Laurent exponent/coefficient pairs",
        ),
        "jones": (
            "laurent-pairs-json",
            "knot",
            "Canonical Laurent exponent/coefficient pairs",
        ),
        "signature": (
            "integer",
            "oriented-knot",
            "RF/Spherogram convention: right-handed positive trefoil has -2",
        ),
        "murasugi_u_lower": (
            "integer",
            "knot",
            "Rigorous lower bound abs(signature)/2",
        ),
        "alexander_genus_lower": (
            "integer",
            "knot",
            "Lower bound degree(Delta)/2",
        ),
    }
    feature_definitions = {
        "braid_strands": "Number of strands in the supplied source representation",
        "word_length": "Number of non-padding letters in the supplied braid word",
        "writhe": "Signed exponent sum of the supplied braid word",
    }

    # Compute every source representation independently. Agreement is required
    # before a value is promoted to the knot-level dictionary.
    computed: dict[str, dict[str, tuple[str, str, str]]] = {}
    for index, representation_id in enumerate(sorted(by_representation), 1):
        if representation_id not in representations:
            raise ValueError(f"missing braid payload for {representation_id}")
        word, strands = representations[representation_id]
        name = by_representation[representation_id][0].removeprefix("knot:")
        crossing_match = re.match(r"^(\d+)(?:_|[an])", name)
        provenance = "RF bundled table value with source hash in parent sidecar"
        values = {
            invariant_id: (value_type, value, provenance)
            for invariant_id, (value_type, value) in source_invariants.get(name, {}).items()
        }
        if "alexander" in values:
            alexander_pairs = json.loads(values["alexander"][1])
        else:
            alexander_pairs = to_pairs(alexander_polynomial(word, strands))
            alexander_text = canonical_json(alexander_pairs)
            values["alexander"] = (
                "laurent-pairs-json",
                alexander_text,
                "exactly recomputed from the supplied braid representation",
            )
            values["determinant"] = (
                "integer",
                str(abs(sum(coefficient * (-1) ** (exponent % 2) for exponent, coefficient in alexander_pairs))),
                "exactly derived from the recomputed Alexander polynomial",
            )
        genus_lower = max((exponent for exponent, _ in alexander_pairs), default=0) // 2
        values["alexander_genus_lower"] = (
            "integer",
            str(genus_lower),
            "exactly derived from the Alexander polynomial degree",
        )
        if crossing_match:
            values["crossing_number_catalogue"] = (
                "integer",
                crossing_match.group(1),
                "canonical catalogue identifier",
            )
        signature_value = signature(word, strands)
        if signature_value is not None:
            values["signature"] = (
                "integer",
                str(signature_value),
                "exactly recomputed by RF/Spherogram from the supplied braid",
            )
            values["murasugi_u_lower"] = (
                "integer",
                str(abs(signature_value) // 2),
                "Murasugi bound derived from the recomputed signature",
            )
        computed[representation_id] = values
        if index % 500 == 0:
            print(f"computed {index}/{len(by_representation)}", flush=True)

    knot_values: list[tuple[str, str, str, str, str]] = []
    overrides: list[tuple[str, str, str, str, str]] = []
    conflicts: list[tuple[str, str, int, str]] = []
    for knot_id, representation_ids in sorted(by_knot.items()):
        invariant_ids = sorted(
            set().union(*(computed[identity].keys() for identity in representation_ids))
        )
        for invariant_id in invariant_ids:
            observed = {
                computed[identity][invariant_id][:2]
                for identity in representation_ids
                if invariant_id in computed[identity]
            }
            if len(observed) == 1:
                value_type, value = next(iter(observed))
                provenances = sorted(
                    {
                        computed[identity][invariant_id][2]
                        for identity in representation_ids
                        if invariant_id in computed[identity]
                    }
                )
                knot_values.append(
                    (
                        knot_id,
                        invariant_id,
                        value_type,
                        value,
                        "; ".join(provenances),
                    )
                )
                continue
            conflicts.append(
                (
                    knot_id,
                    invariant_id,
                    len(observed),
                    canonical_json(sorted(observed)),
                )
            )
            for identity in representation_ids:
                if invariant_id not in computed[identity]:
                    continue
                value_type, value, _ = computed[identity][invariant_id]
                overrides.append(
                    (
                        identity,
                        invariant_id,
                        value_type,
                        value,
                        "representation-specific value retained because knot-level sources disagree",
                    )
                )

    temp = args.output.with_name(args.output.name + f".tmp-{os.getpid()}")
    connection = sqlite3.connect(temp)
    connection.executescript(
        """
        PRAGMA journal_mode=OFF;
        PRAGMA synchronous=OFF;
        CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;
        CREATE TABLE knot_ids(
            knot_id TEXT PRIMARY KEY,
            canonical_name TEXT NOT NULL UNIQUE,
            identifier_scheme TEXT NOT NULL
        ) WITHOUT ROWID;
        CREATE TABLE representation_graph_map(
            representation_id TEXT PRIMARY KEY,
            stopping_key BLOB CHECK(stopping_key IS NULL OR length(stopping_key)=32),
            graph_node_id INTEGER,
            graph_u_upper INTEGER,
            status TEXT NOT NULL
        ) WITHOUT ROWID;
        CREATE INDEX representation_graph_by_key
            ON representation_graph_map(stopping_key);
        CREATE TABLE representation_knot_map(
            representation_id TEXT PRIMARY KEY,
            knot_id TEXT NOT NULL,
            evidence_count INTEGER NOT NULL CHECK(evidence_count>0),
            mapping_status TEXT NOT NULL,
            FOREIGN KEY(knot_id) REFERENCES knot_ids(knot_id)
        ) WITHOUT ROWID;
        CREATE INDEX representation_knot_by_knot
            ON representation_knot_map(knot_id);
        CREATE TABLE graph_vertex_knot_map(
            stopping_key BLOB PRIMARY KEY CHECK(length(stopping_key)=32),
            knot_id TEXT NOT NULL,
            supporting_representations INTEGER NOT NULL CHECK(supporting_representations>0),
            FOREIGN KEY(knot_id) REFERENCES knot_ids(knot_id)
        ) WITHOUT ROWID;
        CREATE TABLE invariant_definitions(
            invariant_id TEXT PRIMARY KEY,
            value_type TEXT NOT NULL,
            scope TEXT NOT NULL,
            definition TEXT NOT NULL
        ) WITHOUT ROWID;
        CREATE TABLE knot_invariant_values(
            knot_id TEXT NOT NULL,
            invariant_id TEXT NOT NULL,
            value_type TEXT NOT NULL,
            value_text TEXT NOT NULL,
            value_sha256 BLOB NOT NULL CHECK(length(value_sha256)=32),
            provenance TEXT NOT NULL,
            PRIMARY KEY(knot_id,invariant_id),
            FOREIGN KEY(knot_id) REFERENCES knot_ids(knot_id),
            FOREIGN KEY(invariant_id) REFERENCES invariant_definitions(invariant_id)
        ) WITHOUT ROWID;
        CREATE INDEX knot_invariants_by_value
            ON knot_invariant_values(invariant_id,value_text,knot_id);
        CREATE TABLE representation_invariant_overrides(
            representation_id TEXT NOT NULL,
            invariant_id TEXT NOT NULL,
            value_type TEXT NOT NULL,
            value_text TEXT NOT NULL,
            value_sha256 BLOB NOT NULL CHECK(length(value_sha256)=32),
            provenance TEXT NOT NULL,
            PRIMARY KEY(representation_id,invariant_id),
            FOREIGN KEY(invariant_id) REFERENCES invariant_definitions(invariant_id)
        ) WITHOUT ROWID;
        CREATE TABLE invariant_conflicts(
            knot_id TEXT NOT NULL,
            invariant_id TEXT NOT NULL,
            distinct_values INTEGER NOT NULL,
            observed_values_json TEXT NOT NULL,
            PRIMARY KEY(knot_id,invariant_id)
        ) WITHOUT ROWID;
        CREATE TABLE representation_feature_definitions(
            feature_id TEXT PRIMARY KEY,
            definition TEXT NOT NULL
        ) WITHOUT ROWID;
        CREATE TABLE representation_feature_values(
            representation_id TEXT NOT NULL,
            feature_id TEXT NOT NULL,
            value_integer INTEGER NOT NULL,
            PRIMARY KEY(representation_id,feature_id)
        ) WITHOUT ROWID;
        CREATE VIEW representation_invariant_map AS
            SELECT m.representation_id,v.invariant_id,v.value_type,v.value_text,
                   v.value_sha256,'knot_dictionary' AS source_scope
            FROM representation_knot_map m
            JOIN knot_invariant_values v USING(knot_id)
            WHERE NOT EXISTS (
                SELECT 1 FROM representation_invariant_overrides o
                WHERE o.representation_id=m.representation_id
                  AND o.invariant_id=v.invariant_id
            )
            UNION ALL
            SELECT representation_id,invariant_id,value_type,value_text,
                   value_sha256,'representation_override'
            FROM representation_invariant_overrides;
        CREATE VIEW invariant_knot_map AS
            SELECT invariant_id,value_type,value_text,value_sha256,knot_id
            FROM knot_invariant_values;
        CREATE VIEW invariant_representation_map AS
            SELECT i.invariant_id,i.value_type,i.value_text,i.value_sha256,
                   i.representation_id,k.knot_id,g.stopping_key,g.graph_node_id
            FROM representation_invariant_map i
            LEFT JOIN representation_knot_map k USING(representation_id)
            LEFT JOIN representation_graph_map g USING(representation_id);
        CREATE VIEW representation_full_map AS
            SELECT g.representation_id,g.stopping_key,g.graph_node_id,g.graph_u_upper,
                   g.status,k.knot_id
            FROM representation_graph_map g
            LEFT JOIN representation_knot_map k USING(representation_id);
        """
    )
    metadata = {
        "schema": "unknotdb-lookup-maps-v1",
        "source_sidecar_sha256": file_sha256(args.source_sidecar),
        "graph_snapshot_sha256": source_meta["snapshot_sha256"],
        "proof_status": "metadata-only-not-part-of-proof-graph",
        "knot_id_scheme": "knot:<canonical-catalogue-name>-v1",
        "invariant_policy": "promote-only-on-exact-agreement-across-mapped-representations",
    }
    for index, path in enumerate(args.corpus_json):
        metadata[f"corpus_{index}_sha256"] = file_sha256(path)
    connection.executemany("INSERT INTO meta VALUES (?,?)", sorted(metadata.items()))
    connection.executemany(
        "INSERT INTO knot_ids VALUES (?,?,?)",
        [
            (knot_id, knot_id.removeprefix("knot:"), "canonical-catalogue-name-v1")
            for knot_id in sorted(by_knot)
        ],
    )
    connection.executemany(
        "INSERT INTO representation_graph_map VALUES (?,?,?,?,?)",
        [
            (
                identity,
                bytes.fromhex(key) if key else None,
                node_id,
                u_upper,
                status,
            )
            for identity, key, node_id, u_upper, status in graph_rows
        ],
    )
    connection.executemany(
        "INSERT INTO representation_knot_map VALUES (?,?,?,?)",
        [
            (identity, knot_id, evidence_count, "unique-provenance-backed")
            for identity, (knot_id, evidence_count) in sorted(by_representation.items())
        ],
    )
    vertex_support: dict[bytes, dict[str, int]] = defaultdict(lambda: defaultdict(int))
    graph_by_rep = {row[0]: row[1] for row in graph_rows}
    for identity, (knot_id, _) in by_representation.items():
        key = graph_by_rep.get(identity)
        if key:
            vertex_support[bytes.fromhex(key)][knot_id] += 1
    vertex_rows = []
    for key, support in vertex_support.items():
        if len(support) != 1:
            raise ValueError(f"canonical graph vertex has conflicting knot IDs: {key.hex()}")
        knot_id, count = next(iter(support.items()))
        vertex_rows.append((key, knot_id, count))
    connection.executemany("INSERT INTO graph_vertex_knot_map VALUES (?,?,?)", vertex_rows)
    connection.executemany(
        "INSERT INTO invariant_definitions VALUES (?,?,?,?)",
        [
            (invariant_id, *definition)
            for invariant_id, definition in sorted(invariant_definitions.items())
        ],
    )
    connection.executemany(
        "INSERT INTO knot_invariant_values VALUES (?,?,?,?,?,?)",
        [
            (
                knot_id,
                invariant_id,
                value_type,
                value,
                value_sha256(invariant_id, value_type, value),
                provenance,
            )
            for knot_id, invariant_id, value_type, value, provenance in knot_values
        ],
    )
    connection.executemany(
        "INSERT INTO representation_invariant_overrides VALUES (?,?,?,?,?,?)",
        [
            (
                identity,
                invariant_id,
                value_type,
                value,
                value_sha256(invariant_id, value_type, value),
                provenance,
            )
            for identity, invariant_id, value_type, value, provenance in overrides
        ],
    )
    connection.executemany("INSERT INTO invariant_conflicts VALUES (?,?,?,?)", conflicts)
    connection.executemany(
        "INSERT INTO representation_feature_definitions VALUES (?,?)",
        sorted(feature_definitions.items()),
    )
    feature_rows = []
    for identity, (word, strands) in representations.items():
        feature_rows.extend(
            (
                (identity, "braid_strands", strands),
                (identity, "word_length", len(word)),
                (identity, "writhe", sum(word)),
            )
        )
    connection.executemany(
        "INSERT INTO representation_feature_values VALUES (?,?,?)", feature_rows
    )
    connection.execute("PRAGMA optimize")
    integrity = connection.execute("PRAGMA integrity_check").fetchone()[0]
    if integrity != "ok":
        raise RuntimeError(f"lookup-map integrity failure: {integrity}")
    counts = {
        "representations": connection.execute(
            "SELECT count(*) FROM representation_graph_map"
        ).fetchone()[0],
        "representation_knot_mappings": connection.execute(
            "SELECT count(*) FROM representation_knot_map"
        ).fetchone()[0],
        "graph_vertex_knot_mappings": connection.execute(
            "SELECT count(*) FROM graph_vertex_knot_map"
        ).fetchone()[0],
        "knot_ids": connection.execute("SELECT count(*) FROM knot_ids").fetchone()[0],
        "knot_invariant_values": connection.execute(
            "SELECT count(*) FROM knot_invariant_values"
        ).fetchone()[0],
        "effective_representation_invariants": connection.execute(
            "SELECT count(*) FROM representation_invariant_map"
        ).fetchone()[0],
        "conflicts": connection.execute(
            "SELECT count(*) FROM invariant_conflicts"
        ).fetchone()[0],
        "overrides": connection.execute(
            "SELECT count(*) FROM representation_invariant_overrides"
        ).fetchone()[0],
    }
    connection.commit()
    connection.close()
    os.replace(temp, args.output)

    report = f"""# Unknot DB lookup maps v1

- Sidecar: `{args.output}` ({args.output.stat().st_size:,} bytes)
- Source representations: {counts['representations']:,}
- `representation → knot_id`: {counts['representation_knot_mappings']:,}
- `canonical graph vertex → knot_id`: {counts['graph_vertex_knot_mappings']:,}
- Stable knot IDs: {counts['knot_ids']:,}
- Deduplicated knot-level invariant values: {counts['knot_invariant_values']:,}
- Effective `representation → invariant` rows exposed by the view: {counts['effective_representation_invariants']:,}
- Cross-representation invariant conflicts: {counts['conflicts']:,}
- Representation-specific overrides: {counts['overrides']:,}

Large polynomial values are stored once per `knot_id`; `representation_invariant_map` is a view, so it behaves like the requested map without copying Alexander/Jones data for every representation. Values are promoted to knot scope only when all available provenance-bearing values agree across mapped source representations. Signatures and missing Alexander/determinant values were recomputed; bundled Alexander/Jones/determinant values retain their table provenance. Any future disagreement is retained in `invariant_conflicts` and `representation_invariant_overrides` rather than overwritten.

The reverse maps `invariant_knot_map` and `invariant_representation_map` use the indexed posting key `(invariant_id,value_text)`. Multi-invariant lookup intersects postings at query time; no quadratic table of every invariant pair is stored.

## Lookup examples

```sql
SELECT knot_id FROM representation_knot_map WHERE representation_id=?1;
SELECT lower(hex(stopping_key)),graph_node_id FROM representation_graph_map WHERE representation_id=?1;
SELECT invariant_id,value_text FROM representation_invariant_map WHERE representation_id=?1;
SELECT knot_id FROM graph_vertex_knot_map WHERE stopping_key=?1;
SELECT knot_id FROM invariant_knot_map WHERE invariant_id=?1 AND value_text=?2;
```

This layer is metadata-only and snapshot-pinned. It cannot modify proof edges, `U_upper`, or the independent validator's conclusions.
"""
    args.report.write_text(report)
    print(" ".join(f"{key}={value}" for key, value in counts.items()))


if __name__ == "__main__":
    main()
