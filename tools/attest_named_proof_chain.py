#!/usr/bin/env python3
"""Atomically attach externally attested knot names to a verified proof chain."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import sqlite3
import time
from pathlib import Path
from typing import Any

from build_graph_identification_sidecar import (
    add_evidence,
    canonical_json,
    file_sha256,
)


def require_sha256(path: Path, expected: str) -> None:
    actual = file_sha256(path)
    if actual != expected:
        raise ValueError(f"SHA-256 mismatch for {path}: {actual} != {expected}")


def state_word(cube: dict[str, Any], mask: int) -> list[int]:
    word = list(map(int, cube["base_word"]))
    positions = list(map(int, cube["marked_positions"]))
    for bit, position in enumerate(positions):
        if mask & (1 << bit):
            word[position] = -word[position]
    return word


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--graph", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    args = parser.parse_args()
    if args.output.exists() or args.report.exists():
        raise FileExistsError("output artifact already exists")

    started = time.monotonic()
    manifest = json.loads(args.manifest.read_text())
    if manifest.get("format") != "unknotdb-named-proof-chain-attestation-v0":
        raise ValueError("unsupported chain manifest")
    require_sha256(args.graph, manifest["graph"]["sha256"])
    cube_path = Path(manifest["wang_zhang_cube"]["corpus"])
    cube_import_path = Path(manifest["wang_zhang_cube"]["import_manifest"])
    require_sha256(cube_path, manifest["wang_zhang_cube"]["corpus_sha256"])
    require_sha256(
        cube_import_path, manifest["wang_zhang_cube"]["import_manifest_sha256"]
    )
    cube = json.loads(cube_path.read_text())

    import snappy

    graph = sqlite3.connect(f"file:{args.graph}?mode=ro", uri=True)
    graph.row_factory = sqlite3.Row
    states = {item["mask"]: item for item in manifest["states"]}
    checks: list[dict[str, Any]] = []
    for mask_text, item in states.items():
        key = bytes.fromhex(item["rep_key"])
        row = graph.execute(
            """
            SELECT k.node_id,n.u_upper_bound,r.encoding
            FROM node_keys k JOIN nodes n USING(node_id)
            JOIN representations r USING(node_id) WHERE k.rep_key=?
            """,
            (key,),
        ).fetchone()
        if row is None or (row["node_id"], row["u_upper_bound"]) != (
            item["node_id"],
            item["u_upper"],
        ):
            raise ValueError(f"graph state mismatch for mask {mask_text}: {row}")
        check: dict[str, Any] = {
            "mask": mask_text,
            "node_id": item["node_id"],
            "rep_key": item["rep_key"],
            "u_upper": item["u_upper"],
        }
        if "dt_code" in item:
            observed = snappy.Link(
                braid_closure=state_word(cube, int(mask_text, 16))
            )
            reference = snappy.Link("DT:" + str(item["dt_code"]))
            matched = bool(
                observed.exterior().is_isometric_to(reference.exterior())
            )
            if not matched:
                raise ValueError(f"SnapPy identification failed for {item['knot_id']}")
            check["snappy_isometric_to_dt"] = True
            check["observed_identify"] = [
                str(value) for value in observed.exterior().identify()
            ]
            check["reference_identify"] = [
                str(value) for value in reference.exterior().identify()
            ]
        checks.append(check)

    for edge in manifest["edges"]:
        source = states[edge["source_mask"]]
        target = states[edge["target_mask"]]
        row = graph.execute(
            """
            SELECT edge_id,source_node,target_node,cc_cost
            FROM edges WHERE edge_id=?
            """,
            (edge["edge_id"],),
        ).fetchone()
        expected = (
            edge["edge_id"],
            source["node_id"],
            target["node_id"],
            edge["cc_cost"],
        )
        if row is None or tuple(row) != expected:
            raise ValueError(f"graph edge mismatch: {tuple(row) if row else None} != {expected}")
        active = graph.execute(
            "SELECT next_unknot_edge FROM nodes WHERE node_id=?",
            (source["node_id"],),
        ).fetchone()[0]
        if active != edge["edge_id"]:
            raise ValueError(f"edge {edge['edge_id']} is not the active U route")
    graph.close()

    parent_sha256 = file_sha256(args.input)
    temp = args.output.with_name(args.output.name + f".tmp-{os.getpid()}")
    shutil.copyfile(args.input, temp)
    db = sqlite3.connect(temp)
    db.execute("PRAGMA foreign_keys=ON")
    graph_sha = db.execute(
        "SELECT value FROM meta WHERE key='graph_snapshot_sha256'"
    ).fetchone()
    if graph_sha != (manifest["graph"]["sha256"],):
        raise ValueError(f"sidecar graph provenance mismatch: {graph_sha}")

    promoted = 0
    manifest_sha256 = file_sha256(args.manifest)
    for item in manifest["states"]:
        if item["canonical_name"] == "0_1":
            continue
        db.execute(
            "INSERT OR IGNORE INTO knot_ids VALUES (?,?,?)",
            (
                item["knot_id"],
                item["canonical_name"],
                "canonical-catalogue-name-v1",
            ),
        )
        evidence = add_evidence(
            db,
            "attested",
            "published-dt-and-snappy-isometry-to-replay-verified-chain-state",
            manifest["paper"]["url"],
            manifest["paper"]["sha256"],
            f"{manifest['paper']['pointer']};mask={item['mask']};node={item['node_id']}",
            {
                "manifest": str(args.manifest),
                "manifest_sha256": manifest_sha256,
                "graph_snapshot_sha256": manifest["graph"]["sha256"],
                "cube_corpus_sha256": manifest["wang_zhang_cube"]["corpus_sha256"],
                "mask": item["mask"],
                "node_id": item["node_id"],
                "rep_key": item["rep_key"],
                "dt_code": item.get("dt_code"),
                "snappy_name": item["snappy_name"],
                "snappy_version": snappy.__version__,
                "trust": "external-knot-name-attestation-over-replay-verified-proof-edge",
            },
        )
        existing = db.execute(
            "SELECT knot_id FROM graph_vertex_knot_map WHERE rep_key=?",
            (bytes.fromhex(item["rep_key"]),),
        ).fetchone()
        if existing is not None and existing != (item["knot_id"],):
            raise RuntimeError(f"conflicting mapping for node {item['node_id']}: {existing}")
        db.execute(
            "INSERT OR IGNORE INTO graph_vertex_knot_map VALUES (?,?,?,?,?,?)",
            (
                bytes.fromhex(item["rep_key"]),
                item["knot_id"],
                None,
                "attested",
                "attested-published-dt-snappy-proof-chain-state",
                evidence,
            ),
        )
        promoted += db.execute("SELECT changes()").fetchone()[0]

    mapped_graph_vertices = db.execute(
        "SELECT count(*) FROM graph_vertex_knot_map"
    ).fetchone()[0]
    effective_source_mappings = db.execute(
        "SELECT count(*) FROM effective_representation_knot_map"
    ).fetchone()[0]
    postings = db.execute(
        "SELECT count(*) FROM knot_representation_postings"
    ).fetchone()[0]
    db.executemany(
        "INSERT OR REPLACE INTO meta VALUES (?,?)",
        (
            ("named_chain_parent_sha256", parent_sha256),
            ("named_chain_manifest_sha256", manifest_sha256),
            ("named_chain_checks", canonical_json(checks)),
            ("count_mapped_graph_vertices", str(mapped_graph_vertices)),
            ("count_source_effective", str(effective_source_mappings)),
            ("count_postings", str(postings)),
        ),
    )
    integrity = db.execute("PRAGMA integrity_check").fetchone()[0]
    foreign_keys = db.execute("PRAGMA foreign_key_check").fetchall()
    if integrity != "ok" or foreign_keys:
        raise RuntimeError(f"integrity={integrity} foreign_keys={foreign_keys[:3]}")
    db.commit()
    db.close()
    os.replace(temp, args.output)

    output_sha256 = file_sha256(args.output)
    elapsed = time.monotonic() - started
    args.report.write_text(
        "# Brittenham-Hermiller named proof-chain attestation\n\n"
        f"- Parent: `{args.input}` (SHA-256 `{parent_sha256}`)\n"
        f"- Graph: `{args.graph}` (SHA-256 `{manifest['graph']['sha256']}`)\n"
        f"- Manifest: `{args.manifest}` (SHA-256 `{manifest_sha256}`)\n"
        f"- Output: `{args.output}` ({args.output.stat().st_size:,} bytes; SHA-256 `{output_sha256}`)\n"
        f"- Promoted graph mappings: {promoted}\n"
        f"- Mapped graph vertices: {mapped_graph_vertices:,}\n"
        f"- Effective source mappings: {effective_source_mappings:,}\n"
        f"- Reverse postings: {postings:,}\n"
        f"- Elapsed: {elapsed:.1f} seconds\n"
        "- SQLite integrity and foreign keys: `ok`\n\n"
        "The knot names are external attestations. The graph edges, CC costs, and U route are independently replay-validated facts in the pinned core snapshot.\n\n"
        "```json\n" + json.dumps(checks, indent=2, sort_keys=True) + "\n```\n"
    )
    print(
        canonical_json(
            {
                "promoted_graph_mappings": promoted,
                "mapped_graph_vertices": mapped_graph_vertices,
                "effective_source_mappings": effective_source_mappings,
                "postings": postings,
                "elapsed_seconds": elapsed,
                "sha256": output_sha256,
            }
        )
    )


if __name__ == "__main__":
    main()
