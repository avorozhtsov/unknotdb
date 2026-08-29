#!/usr/bin/env python3
"""Audit DGKT lower-bound rows against named replay-verified graph routes."""

from __future__ import annotations

import argparse
import hashlib
import json
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
    parser.add_argument("--lower-bounds", type=Path, required=True)
    parser.add_argument("--identifications", type=Path, required=True)
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--exact-unresolved", type=Path, required=True)
    parser.add_argument("--range-unresolved", type=Path, required=True)
    args = parser.parse_args()

    db = sqlite3.connect(f"file:{args.identifications}?mode=ro", uri=True)
    db.execute("ATTACH DATABASE ? AS lb", (str(args.lower_bounds),))
    db.execute("ATTACH DATABASE ? AS proof", (str(args.graph),))
    rows = [
        {
            "knot_id": knot,
            "u_lower": lower,
            "retained_upper": upper,
            "interval_exact": bool(exact),
            "best_named_graph_u": graph_u,
            "status": (
                "covered"
                if graph_u is not None and graph_u <= upper
                else "mapped-too-long"
                if graph_u is not None
                else "unmapped"
            ),
        }
        for knot, lower, upper, exact, graph_u in db.execute(
            """
            WITH best AS (
              SELECT c.knot_id,min(n.u_upper_bound) AS graph_u
              FROM lb.lower_bound_claims c
              JOIN graph_vertex_knot_map m USING(knot_id)
              JOIN graph_vertices v USING(rep_key)
              JOIN proof.nodes n ON n.node_id=v.node_id
              GROUP BY c.knot_id
            )
            SELECT c.knot_id,c.u_lower,c.retained_upper,c.interval_exact,b.graph_u
            FROM lb.lower_bound_claims c LEFT JOIN best b USING(knot_id)
            ORDER BY c.crossing_number,c.knot_id
            """
        )
    ]
    db.close()
    unresolved = [row for row in rows if row["status"] != "covered"]
    exact = [row["knot_id"] for row in unresolved if row["interval_exact"]]
    ranges = [row["knot_id"] for row in unresolved if not row["interval_exact"]]
    args.exact_unresolved.write_text("\n".join(exact) + "\n")
    args.range_unresolved.write_text("\n".join(ranges) + "\n")
    counts = {
        status: sum(row["status"] == status for row in rows)
        for status in ("covered", "mapped-too-long", "unmapped")
    }
    report = {
        "format": "unknotdb-dgkt-witness-coverage-report-v0",
        "inputs": {
            "lower_bounds_sha256": sha256(args.lower_bounds),
            "identifications_sha256": sha256(args.identifications),
            "graph_sha256": sha256(args.graph),
        },
        "summary": {
            "claims": len(rows),
            **counts,
            "unresolved_exact": len(exact),
            "unresolved_range": len(ranges),
        },
        "rows": rows,
    }
    args.report.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps(report["summary"], sort_keys=True))


if __name__ == "__main__":
    main()
