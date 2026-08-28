#!/usr/bin/env python3
"""Atomically repin lookup maps to a rebased identification sidecar and graph."""

from __future__ import annotations

import argparse
import hashlib
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


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--identifications", type=Path, required=True)
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        raise FileExistsError(args.output)
    graph_sha256 = file_sha256(args.graph)
    identification_sha256 = file_sha256(args.identifications)
    identification = sqlite3.connect(f"file:{args.identifications}?mode=ro", uri=True)
    identification_meta = dict(identification.execute("SELECT key,value FROM meta"))
    if identification_meta.get("graph_snapshot_sha256") != graph_sha256:
        raise ValueError("identification sidecar pins another graph")

    representation_rows = list(
        identification.execute(
            """
            SELECT representation_id,stopping_key,graph_node_id,graph_u_upper,graph_status
            FROM representations ORDER BY representation_id
            """
        )
    )
    vertex_rows = list(
        identification.execute(
            "SELECT rep_key,knot_id FROM graph_vertex_knot_map ORDER BY rep_key"
        )
    )
    identification.close()

    temporary = args.output.with_name(f"{args.output.name}.tmp-{os.getpid()}")
    shutil.copyfile(args.input, temporary)
    lookup = sqlite3.connect(temporary)
    try:
        known_representations = {
            row[0]
            for row in lookup.execute(
                "SELECT representation_id FROM representation_graph_map"
            )
        }
        incoming_representations = {row[0] for row in representation_rows}
        if known_representations != incoming_representations:
            raise ValueError("representation corpus changed during lookup rebase")
        lookup.execute("DELETE FROM representation_graph_map")
        lookup.executemany(
            "INSERT INTO representation_graph_map VALUES (?,?,?,?,?)",
            representation_rows,
        )
        known_knots = {
            knot_id for (knot_id,) in lookup.execute("SELECT knot_id FROM knot_ids")
        }
        vertex_rows = [row for row in vertex_rows if row[1] in known_knots]
        previous_support = dict(
            lookup.execute(
                "SELECT stopping_key,supporting_representations FROM graph_vertex_knot_map"
            )
        )
        lookup.execute("DELETE FROM graph_vertex_knot_map")
        lookup.executemany(
            "INSERT INTO graph_vertex_knot_map VALUES (?,?,?)",
            (
                (bytes(key), knot, previous_support.get(bytes(key), 1))
                for key, knot in vertex_rows
            ),
        )
        lookup.executemany(
            "INSERT OR REPLACE INTO meta VALUES (?,?)",
            (
                ("graph_snapshot_sha256", graph_sha256),
                ("mapping_sidecar_sha256", identification_sha256),
                ("rebase_parent_lookup_sha256", file_sha256(args.input)),
            ),
        )
        lookup.execute("PRAGMA optimize")
        integrity = lookup.execute("PRAGMA integrity_check").fetchone()[0]
        foreign_keys = lookup.execute("PRAGMA foreign_key_check").fetchall()
        if integrity != "ok" or foreign_keys:
            raise ValueError(f"lookup rebase failed: {integrity}, {foreign_keys[:3]}")
        lookup.commit()
        lookup.close()
        os.replace(temporary, args.output)
    finally:
        try:
            lookup.close()
        except sqlite3.Error:
            pass
        temporary.unlink(missing_ok=True)
    print(
        f"output={args.output} graph_sha256={graph_sha256} "
        f"representations={len(representation_rows)} graph_mappings={len(vertex_rows)}"
    )


if __name__ == "__main__":
    main()
