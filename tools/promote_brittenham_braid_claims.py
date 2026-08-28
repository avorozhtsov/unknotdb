#!/usr/bin/env python3
"""Atomically promote exact replayed Brittenham braid claims in a sidecar."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import shutil
import sqlite3
from pathlib import Path


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def improved_rows(paths: list[Path]) -> list[dict[str, str]]:
    rows: list[dict[str, str]] = []
    for path in paths:
        with path.open() as stream:
            lines = (line for line in stream if line.startswith("result\t"))
            reader = csv.reader(lines, delimiter="\t")
            for fields in reader:
                if fields[4] != "improved":
                    continue
                rows.append(
                    {
                        "claim_id": fields[1],
                        "source_knot": fields[2],
                        "target_knot": fields[3],
                        "source_key": fields[5],
                        "target_key": fields[6],
                        "new_u": fields[8],
                        "program_sha256": fields[9],
                        "manifest": str(path),
                    }
                )
    return rows


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--import-manifest", type=Path, action="append", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    args = parser.parse_args()
    for output in (args.output, args.manifest):
        if output.exists():
            raise FileExistsError(output)

    graph_sha256 = file_sha256(args.graph)
    rows = improved_rows(args.import_manifest)
    temporary = args.output.with_name(f"{args.output.name}.tmp-{os.getpid()}")
    shutil.copyfile(args.input, temporary)
    sidecar = sqlite3.connect(temporary)
    graph = sqlite3.connect(f"file:{args.graph}?mode=ro", uri=True)
    promoted = []
    try:
        for row in rows:
            source_key = bytes.fromhex(row["source_key"])
            target_key = bytes.fromhex(row["target_key"])
            edge = graph.execute(
                """
                SELECT e.edge_id,e.source_node,e.target_node,nt.u_upper_bound,
                       hex(p.program_sha256)
                FROM edges e
                JOIN node_keys ks ON ks.node_id=e.source_node
                JOIN node_keys kt ON kt.node_id=e.target_node
                JOIN programs p ON p.program_id=e.program_id
                JOIN nodes nt ON nt.node_id=e.target_node
                WHERE ks.rep_key=? AND kt.rep_key=? AND e.cc_cost=1
                """,
                (source_key, target_key),
            ).fetchall()
            if len(edge) != 1:
                raise ValueError(
                    f"claim {row['claim_id']} has {len(edge)} matching edges"
                )
            edge_id, source_node, target_node, target_u, stored_program_sha256 = edge[0]
            stored_program_sha256 = stored_program_sha256.lower()
            claim = sidecar.execute(
                """
                SELECT a.status,ks.canonical_id,kt.canonical_id,a.source_knot_pk,a.target_knot_pk
                FROM adjacency_claims a
                JOIN knots ks ON ks.knot_pk=a.source_knot_pk
                JOIN knots kt ON kt.knot_pk=a.target_knot_pk
                WHERE a.claim_id=?
                """,
                (bytes.fromhex(row["claim_id"]),),
            ).fetchone()
            if claim is None or claim[1:3] != (row["source_knot"], row["target_knot"]):
                raise ValueError(f"claim identity mismatch: {row['claim_id']}")
            source_knot_pk, target_knot_pk = claim[3], claim[4]
            sidecar.execute(
                """
                UPDATE adjacency_claims
                SET status='replay_verified',claim_scope='normalized_representation',
                    source_stopping_key=?,target_stopping_key=?,
                    witness_uri=?,witness_sha256=?,graph_snapshot_sha256=?,graph_edge_id=?
                WHERE claim_id=? AND status='diagram_attested'
                """,
                (
                    source_key,
                    target_key,
                    f"unknotdb-program-sha256:{stored_program_sha256}",
                    stored_program_sha256,
                    graph_sha256,
                    edge_id,
                    bytes.fromhex(row["claim_id"]),
                ),
            )
            if sidecar.execute("SELECT changes()").fetchone()[0] != 1:
                raise ValueError(f"claim was not promotable: {row['claim_id']}")
            for knot_pk, key, node_id, suffix, u_value in (
                (source_knot_pk, source_key, source_node, "source", int(row["new_u"])),
                (target_knot_pk, target_key, target_node, "target", int(target_u)),
            ):
                sidecar.execute(
                    """
                    INSERT OR IGNORE INTO graph_knot_links(
                      knot_pk,stopping_key,graph_node_id,graph_u_upper,
                      evidence_class,representation_id
                    ) VALUES (?,?,?,?,?,?)
                    """,
                    (
                        knot_pk,
                        key,
                        node_id,
                        u_value,
                        "attested",
                        f"brittenham-claim:{row['claim_id']}:{suffix}",
                    ),
                )
            promoted.append(
                {
                    **{
                        key: value
                        for key, value in row.items()
                        if key != "program_sha256"
                    },
                    "compiler_program_sha256": row["program_sha256"],
                    "stored_program_sha256": stored_program_sha256,
                    "edge_id": edge_id,
                }
            )
        sidecar.execute("PRAGMA optimize")
        integrity = sidecar.execute("PRAGMA integrity_check").fetchone()[0]
        foreign_keys = sidecar.execute("PRAGMA foreign_key_check").fetchall()
        if integrity != "ok" or foreign_keys:
            raise RuntimeError(
                f"sidecar validation failed: {integrity}, {foreign_keys[:3]}"
            )
        sidecar.commit()
    except Exception:
        temporary.unlink(missing_ok=True)
        raise
    finally:
        graph.close()
        sidecar.close()
    os.replace(temporary, args.output)
    manifest = {
        "format": "unknotdb-brittenham-braid-promotion-v0",
        "parent_sidecar": str(args.input),
        "parent_sidecar_sha256": file_sha256(args.input),
        "graph_snapshot": str(args.graph),
        "graph_snapshot_sha256": graph_sha256,
        "import_manifests": [
            {"path": str(path), "sha256": file_sha256(path)}
            for path in args.import_manifest
        ],
        "promoted": promoted,
        "promoted_count": len(promoted),
        "output": str(args.output),
        "output_sha256": file_sha256(args.output),
        "output_bytes": args.output.stat().st_size,
        "validation": {"integrity_check": "ok", "foreign_key_check_rows": 0},
    }
    args.manifest.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
