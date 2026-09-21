#!/usr/bin/env python3
"""Read-only release preparation; never repin, promote, or publish a database."""

from __future__ import annotations

import argparse
import hashlib
import json
import sqlite3
import subprocess
from pathlib import Path


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def connect(path: Path) -> sqlite3.Connection:
    db = sqlite3.connect(path.resolve().as_uri() + "?mode=ro", uri=True)
    db.execute("PRAGMA query_only=ON")
    if db.execute("PRAGMA integrity_check").fetchone()[0] != "ok":
        raise ValueError(f"integrity check failed: {path}")
    if db.execute("PRAGMA foreign_key_check").fetchall():
        raise ValueError(f"foreign key check failed: {path}")
    return db


def graph_rows(db: sqlite3.Connection) -> dict:
    return dict(db.execute(
        "SELECT k.rep_key,n.u_upper_bound FROM node_keys k JOIN nodes n USING(node_id) "
        "WHERE k.key_kind=0"
    ))


def compare_bounds(old: dict, new: dict) -> dict:
    common = old.keys() & new.keys()
    return {
        "common_canonical_keys": len(common),
        "missing_old_canonical_keys": len(old.keys() - new.keys()),
        "added_canonical_keys": len(new.keys() - old.keys()),
        "newly_finite_upper_bounds": sum(old[k] is None and new[k] is not None for k in common),
        "improved_finite_upper_bounds": sum(
            old[k] is not None and new[k] is not None and new[k] < old[k] for k in common
        ),
        "regressed_upper_bounds": sum(
            old[k] is not None and (new[k] is None or new[k] > old[k]) for k in common
        ),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--previous", type=Path, required=True)
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--identification", type=Path, required=True)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        raise FileExistsError(args.output)
    manifest_path = args.previous / "release.json"
    manifest = json.loads(manifest_path.read_text())
    components = {row["name"]: args.previous / row["name"] for row in manifest["components"]}
    paths = [Path(__file__), manifest_path, args.graph, args.identification,
             args.runtime, *components.values()]
    hashes = {str(p.resolve()): sha(p) for p in paths}
    for row in manifest["components"]:
        if sha(components[row["name"]]) != row["sha256"]:
            raise ValueError(f"previous release component changed: {row['name']}")
    graph_sha = sha(args.graph)
    old = connect(components["proof.sqlite"])
    new = connect(args.graph)
    ids = connect(args.identification)
    try:
        changes = compare_bounds(graph_rows(old), graph_rows(new))
        counts = {
            name: {table: db.execute(f"SELECT count(*) FROM {table}").fetchone()[0]
                   for table in ("nodes", "edges", "programs")}
            for name, db in (("previous", old), ("candidate", new))
        }
        metadata = dict(ids.execute("SELECT key,value FROM meta"))
        disagreements = [
            {"component": key, "prior_knot_id": prior, "snappy_ids": json.loads(names),
             "retained_mapping_count": mapped, "retained_names": retained}
            for key, prior, names, mapped, retained in ids.execute(
                "SELECT lower(hex(a.cc0_component_key)),a.prior_knot_id,"
                "a.snappy_canonical_ids_json,count(m.rep_key),group_concat(distinct m.knot_id) "
                "FROM snappy_prior_label_agreement a "
                "JOIN graph_vertices v USING(cc0_component_key) "
                "LEFT JOIN graph_vertex_knot_map m USING(rep_key) "
                "WHERE a.verdict='disagree' GROUP BY a.cc0_component_key"
            )
        ]
        quarantines = dict(ids.execute(
            "SELECT reason,count(*) FROM snappy_quarantine GROUP BY reason"
        ))
    finally:
        old.close()
        new.close()
        ids.close()
    validation = subprocess.run(
        [str(args.runtime.resolve()), "validate", str(args.graph.resolve())],
        check=True, capture_output=True, text=True,
    )
    component_pins = {}
    for name, path in components.items():
        if name == "proof.sqlite":
            continue
        db = connect(path)
        try:
            component_pins[name] = dict(db.execute("SELECT key,value FROM meta")).get(
                "graph_snapshot_sha256"
            )
        finally:
            db.close()
    blockers = []
    if changes["missing_old_canonical_keys"] or changes["regressed_upper_bounds"]:
        blockers.append("Candidate loses previous keys or regresses recorded U bounds.")
    if metadata.get("graph_snapshot_sha256") != graph_sha:
        blockers.append("Identification sidecar pins another proof graph; rebase and audit required.")
    if any(pin != graph_sha for pin in component_pins.values()):
        blockers.append("Existing release metadata cannot be copied unchanged; rebuild coherent sidecars.")
    if any(row["retained_mapping_count"] for row in disagreements):
        blockers.append("A disputed prior knot identity is still exposed; resolve or quarantine it explicitly.")
    blockers.append("Full new-bundle checksum, dependency, B4 and query gates not yet executed.")
    for path in paths:
        if sha(path) != hashes[str(path.resolve())]:
            raise ValueError(f"input changed during read-only audit: {path}")
    report = {
        "schema": "unknotdb-release-preparation-v1",
        "status": "preflight_completed", "assembly_ready": False, "published": False,
        "inputs": hashes, "previous_version": manifest["version"],
        "candidate_graph_sha256": graph_sha, "graph_counts": counts,
        "canonical_key_comparison": changes,
        "candidate_graph_validation": validation.stdout.strip(),
        "identification_graph_pin": metadata.get("graph_snapshot_sha256"),
        "old_component_graph_pins": component_pins,
        "identification_quarantine_counts": quarantines,
        "prior_label_disagreements": disagreements,
        "release_blockers": blockers,
        "limits": ["Counts are representations, not independent knot types.",
                   "External identifications remain attestations, never proof-graph edges.",
                   "No claim that current embedding training generated these historical improvements.",
                   "L10/L1000 improvements are not measured by U-bound comparison."],
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as stream:
        stream.write(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps(report, sort_keys=True))


if __name__ == "__main__":
    main()
