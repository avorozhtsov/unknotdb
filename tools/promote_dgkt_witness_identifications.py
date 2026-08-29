#!/usr/bin/env python3
"""Attach DGKT knot IDs to replay-covered graph stopping points."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import sqlite3
from pathlib import Path


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--expected-rows", type=int, required=True)
    args = parser.parse_args()
    if args.output.exists():
        raise FileExistsError(args.output)

    graph_sha = sha256(args.graph)
    manifest_sha = sha256(args.manifest)
    graph = sqlite3.connect(f"file:{args.graph}?mode=ro", uri=True)
    graph_keys = {
        bytes(row[0]) for row in graph.execute("SELECT rep_key FROM node_keys")
    }
    graph.close()

    lines = args.manifest.read_text().splitlines()
    header = next(line for line in lines if line.startswith("row\t"))
    columns = header.split("\t")[1:]
    rows = []
    for line_number, line in enumerate(lines, 1):
        if not line.startswith("result\t"):
            continue
        row = dict(zip(columns, line.split("\t")[1:], strict=True))
        if row["status"] not in {
            "improved",
            "already-sufficient",
            "already-sufficient-after-preprocessing",
        }:
            continue
        key = bytes.fromhex(row["source_key"])
        if key not in graph_keys:
            raise ValueError(
                f"manifest source key is absent from graph at line {line_number}"
            )
        rows.append((line_number, row, key))
    if len(rows) != args.expected_rows:
        raise ValueError(
            f"expected {args.expected_rows} covered DGKT rows, found {len(rows)}"
        )

    temporary = args.output.with_name(f"{args.output.name}.tmp-{os.getpid()}")
    shutil.copyfile(args.input, temporary)
    db = sqlite3.connect(temporary)
    db.execute("PRAGMA foreign_keys=ON")
    if (
        dict(db.execute("SELECT key,value FROM meta")).get("graph_snapshot_sha256")
        != graph_sha
    ):
        raise ValueError("identification sidecar is not pinned to the supplied graph")

    inserted = 0
    existing = 0
    for line_number, row, key in rows:
        knot_id = row["knot_id"]
        db.execute(
            "INSERT OR IGNORE INTO knot_ids VALUES (?,?,?)",
            (knot_id, knot_id.removeprefix("knot:"), "knotinfo"),
        )
        payload = {
            "class": "attested",
            "kind": "dgkt-standard-braid-preprocessed-upper-witness",
            "source_path": str(args.manifest),
            "source_sha256": manifest_sha,
            "source_pointer": f"line:{line_number}",
            "details": {
                "knot_id": knot_id,
                "source_key": key.hex(),
                "graph_snapshot_sha256": graph_sha,
                "graph_u_upper": int(row["new_u"]),
                "cc_count": int(row["cc_count"]),
                "identity_boundary": (
                    "named Spherogram standard braid plus replay-verified preprocessing"
                ),
            },
        }
        canonical = json.dumps(payload, sort_keys=True, separators=(",", ":"))
        evidence_id = hashlib.sha256(
            b"UNKNOTDB_IDENTIFICATION_EVIDENCE_V1\0" + canonical.encode()
        ).digest()
        db.execute(
            "INSERT OR IGNORE INTO identification_evidence VALUES (?,?,?,?,?,?,?)",
            (
                evidence_id,
                "attested",
                payload["kind"],
                str(args.manifest),
                manifest_sha,
                f"line:{line_number}",
                json.dumps(payload["details"], sort_keys=True, separators=(",", ":")),
            ),
        )
        current = db.execute(
            "SELECT knot_id FROM graph_vertex_knot_map WHERE rep_key=?", (key,)
        ).fetchone()
        if current is not None:
            if current[0] != knot_id:
                raise ValueError(
                    f"conflicting graph identity for {key.hex()}: {current[0]} vs {knot_id}"
                )
            existing += 1
            continue
        db.execute(
            "INSERT INTO graph_vertex_knot_map VALUES (?,?,?,?,?,?)",
            (
                key,
                knot_id,
                None,
                "attested",
                "attested-dgkt-upper-witness-source",
                evidence_id,
            ),
        )
        inserted += 1

    db.executemany(
        "INSERT OR REPLACE INTO meta VALUES (?,?)",
        (
            ("dgkt_witness_manifest_sha256", manifest_sha),
            ("dgkt_witness_mapped_rows", str(len(rows))),
            ("dgkt_witness_inserted_mappings", str(inserted)),
            ("dgkt_witness_existing_mappings", str(existing)),
        ),
    )
    integrity = db.execute("PRAGMA integrity_check").fetchone()[0]
    foreign_keys = db.execute("PRAGMA foreign_key_check").fetchall()
    if integrity != "ok" or foreign_keys:
        raise ValueError(f"sidecar validation failed: {integrity}, {foreign_keys[:3]}")
    db.commit()
    db.close()
    os.replace(temporary, args.output)
    print(
        json.dumps(
            {
                "mapped_rows": len(rows),
                "inserted": inserted,
                "existing": existing,
                "graph_sha256": graph_sha,
                "manifest_sha256": manifest_sha,
            },
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
