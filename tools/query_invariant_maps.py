#!/usr/bin/env python3
"""Intersect one or more Unknot DB invariant postings."""

from __future__ import annotations

import argparse
import sqlite3
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("sidecar", type=Path)
    parser.add_argument(
        "--equals",
        action="append",
        required=True,
        metavar="INVARIANT=VALUE",
        help="exact canonical value; repeat to intersect two or more invariants",
    )
    parser.add_argument("--limit", type=int, default=100)
    args = parser.parse_args()
    if args.limit <= 0:
        raise ValueError("--limit must be positive")
    wanted = []
    for expression in args.equals:
        if "=" not in expression:
            raise ValueError("--equals must have the form INVARIANT=VALUE")
        invariant_id, value = expression.split("=", 1)
        if not invariant_id or not value:
            raise ValueError("invariant ID and value must be non-empty")
        wanted.append((invariant_id, value))
    if len(set(wanted)) != len(wanted):
        raise ValueError("duplicate invariant constraints")

    values_sql = ",".join("(?,?)" for _ in wanted)
    parameters = [item for pair in wanted for item in pair]
    parameters.extend((len(wanted), args.limit))
    connection = sqlite3.connect(f"file:{args.sidecar}?mode=ro", uri=True)
    rows = connection.execute(
        f"""
        WITH wanted(invariant_id,value_text) AS (VALUES {values_sql}),
        matching_knots AS (
            SELECT v.knot_id
            FROM knot_invariant_values v
            JOIN wanted w USING(invariant_id,value_text)
            GROUP BY v.knot_id
            HAVING count(*)=?
        )
        SELECT m.knot_id,m.representation_id,lower(hex(g.stopping_key)),g.graph_node_id
        FROM matching_knots x
        JOIN representation_knot_map m USING(knot_id)
        LEFT JOIN representation_graph_map g USING(representation_id)
        ORDER BY m.knot_id,m.representation_id
        LIMIT ?
        """,
        parameters,
    )
    print("knot_id\trepresentation_id\tstopping_key\tgraph_node_id")
    for knot_id, representation_id, stopping_key, graph_node_id in rows:
        print(
            f"{knot_id}\t{representation_id}\t{stopping_key or '-'}\t"
            f"{graph_node_id if graph_node_id is not None else '-'}"
        )
    connection.close()


if __name__ == "__main__":
    main()
