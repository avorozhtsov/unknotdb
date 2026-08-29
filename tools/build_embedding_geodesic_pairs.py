#!/usr/bin/env python3
"""Derive exact CC=1/2 pairs from externally pinned exact-U graph routes."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sqlite3
from dataclasses import dataclass
from pathlib import Path

SCHEMA = "unknotdb-embedding-geodesic-pairs-v0"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


@dataclass(frozen=True)
class RouteNode:
    u_upper: int | None
    next_edge: int | None
    next_target: int | None
    next_cc: int | None


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--identification", type=Path, required=True)
    parser.add_argument("--lower-bounds", type=Path, required=True)
    parser.add_argument("--pairs", type=Path, required=True)
    parser.add_argument("--hard-cases", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    graph = sqlite3.connect(f"file:{args.graph}?mode=ro", uri=True)
    identification = sqlite3.connect(
        f"file:{args.identification}?mode=ro", uri=True
    )
    lower_bounds = sqlite3.connect(
        f"file:{args.lower_bounds}?mode=ro", uri=True
    )
    pairs = sqlite3.connect(f"file:{args.pairs}?mode=ro", uri=True)
    hard = sqlite3.connect(f"file:{args.hard_cases}?mode=ro", uri=True)

    pair_graph_sha = pairs.execute(
        "SELECT value FROM meta WHERE key='proof_sha256'"
    ).fetchone()[0]
    if pair_graph_sha != sha256(args.graph):
        raise ValueError("embedding pairs are pinned to another proof graph")
    identification_graph_sha = identification.execute(
        "SELECT value FROM meta WHERE key='graph_snapshot_sha256'"
    ).fetchone()[0]
    if identification_graph_sha != pair_graph_sha:
        raise ValueError("identification sidecar is pinned to another proof graph")

    protected = {
        int(row[0])
        for row in hard.execute(
            "SELECT DISTINCT representation_id FROM hard_case_representations"
        )
    }
    mirror_by_representation = {
        int(row[0]): bytes(row[1])
        for row in pairs.execute(
            "SELECT representation_id,mirror_key FROM representations"
        )
    }
    split_by_representation = {
        int(row[0]): str(row[1])
        for row in pairs.execute(
            "SELECT representation_id,split FROM representations"
        )
    }
    protected_mirror_keys = {
        mirror_by_representation[representation_id]
        for representation_id in protected
    }
    representation_by_mirror = {}
    for representation_id, mirror_key in mirror_by_representation.items():
        representation_by_mirror.setdefault(mirror_key, representation_id)
    representation_by_node = {
        int(node_id): representation_by_mirror[bytes(rep_key)]
        for rep_key, node_id in graph.execute(
            "SELECT rep_key,node_id FROM node_keys WHERE key_kind=0 ORDER BY node_id"
        )
        if bytes(rep_key) in representation_by_mirror
    }

    nodes = {
        int(row[0]): RouteNode(
            int(row[1]) if row[1] is not None else None,
            int(row[2]) if row[2] is not None else None,
            int(row[3]) if row[3] is not None else None,
            int(row[4]) if row[4] is not None else None,
        )
        for row in graph.execute(
            "SELECT node_id,u_upper_bound,next_unknot_edge,next_unknot_target,"
            "next_unknot_cc_cost FROM nodes"
        )
    }
    edge_endpoints = {
        int(row[0]): (int(row[1]), int(row[2]))
        for row in graph.execute("SELECT edge_id,source_node,target_node FROM edges")
    }

    def edge_semantic_length(edge_id: int) -> int | None:
        source_node, target_node = edge_endpoints[edge_id]
        source_representation = representation_by_node.get(source_node)
        target_representation = representation_by_node.get(target_node)
        if source_representation is None or target_representation is None:
            return None
        left, right = sorted((source_representation, target_representation))
        row = pairs.execute(
            "SELECT MIN(witness_semantic_length) FROM ("
            "SELECT left_representation,right_representation,edge_id,"
            "witness_semantic_length FROM pairs UNION ALL "
            "SELECT left_representation,right_representation,edge_id,"
            "witness_semantic_length FROM excluded_cross_split_pairs) "
            "WHERE left_representation=? AND right_representation=? AND edge_id=?",
            (left, right, edge_id),
        ).fetchone()
        return int(row[0]) if row is not None and row[0] is not None else None

    lower_by_knot = {
        str(row[0]): (int(row[1]), str(row[2]), str(row[3]), str(row[4]))
        for row in lower_bounds.execute(
            "SELECT knot_id,u_lower,trust_status,source_id,source_pointer "
            "FROM effective_lower_bounds"
        )
    }
    exact_sources = []
    for node_id, knot_id in identification.execute(
        "SELECT node_id,knot_id FROM graph_vertices JOIN graph_vertex_knot_map "
        "USING(rep_key) ORDER BY node_id"
    ):
        lower = lower_by_knot.get(str(knot_id))
        node = nodes[int(node_id)]
        if lower is not None and node.u_upper == lower[0] and lower[0] > 0:
            exact_sources.append((int(node_id), str(knot_id), lower))

    derived = {}
    skipped_unsupported_edge = 0
    skipped_protected = 0
    skipped_nontrain = 0
    for source_node, knot_id, lower in exact_sources:
        source_representation = representation_by_node.get(source_node)
        if source_representation is None:
            continue
        current = source_node
        cc = 0
        semantic_length = 0
        visited = {current}
        while cc < 2:
            route = nodes[current]
            if (
                route.next_edge is None
                or route.next_target is None
                or route.next_cc is None
            ):
                break
            edge_length = edge_semantic_length(route.next_edge)
            if edge_length is None:
                skipped_unsupported_edge += 1
                break
            semantic_length += edge_length
            cc += route.next_cc
            current = route.next_target
            if current in visited:
                raise ValueError(f"cycle in U route from node {source_node}")
            visited.add(current)
            if cc not in (1, 2):
                continue
            target_representation = representation_by_node.get(current)
            if target_representation is None:
                break
            if (
                source_representation in protected
                or target_representation in protected
                or mirror_by_representation[source_representation]
                in protected_mirror_keys
                or mirror_by_representation[target_representation]
                in protected_mirror_keys
            ):
                skipped_protected += 1
                continue
            if (
                split_by_representation[source_representation] != "train"
                or split_by_representation[target_representation] != "train"
            ):
                skipped_nontrain += 1
                continue
            left, right = sorted((source_representation, target_representation))
            key = (left, right)
            candidate = (
                cc,
                semantic_length,
                source_node,
                current,
                knot_id,
                lower[1],
                lower[2],
                lower[3],
            )
            if key in derived and derived[key][0] != cc:
                raise ValueError(f"inconsistent exact CC labels for pair {key}")
            if key not in derived or semantic_length < derived[key][1]:
                derived[key] = candidate

    temporary = args.output.with_name(f".{args.output.name}.{os.getpid()}.part")
    temporary.unlink(missing_ok=True)
    output = sqlite3.connect(temporary)
    output.executescript(
        """
        CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;
        CREATE TABLE geodesic_pairs(
          left_representation INTEGER NOT NULL,
          right_representation INTEGER NOT NULL,
          distance INTEGER NOT NULL CHECK(distance IN (1,2)),
          witness_semantic_length INTEGER NOT NULL,
          source_node INTEGER NOT NULL,
          target_node INTEGER NOT NULL,
          source_knot_id TEXT NOT NULL,
          lower_bound_trust TEXT NOT NULL,
          lower_bound_source TEXT NOT NULL,
          lower_bound_pointer TEXT NOT NULL,
          derivation TEXT NOT NULL,
          PRIMARY KEY(left_representation,right_representation)
        ) WITHOUT ROWID;
        """
    )
    metadata = {
        "schema": SCHEMA,
        "proof_sha256": pair_graph_sha,
        "pairs_sha256": sha256(args.pairs),
        "identification_sha256": sha256(args.identification),
        "lower_bounds_sha256": sha256(args.lower_bounds),
        "hard_cases_sha256": sha256(args.hard_cases),
        "split_contract": "training-only; protected representations excluded",
    }
    output.executemany("INSERT INTO meta VALUES (?,?)", metadata.items())
    for (left, right), row in sorted(derived.items()):
        output.execute(
            "INSERT INTO geodesic_pairs VALUES (?,?,?,?,?,?,?,?,?,?,?)",
            (
                left,
                right,
                row[0],
                row[1],
                row[2],
                row[3],
                row[4],
                row[5],
                row[6],
                row[7],
                (
                    "Exact source U equals an attested lower bound; shortening this "
                    "route prefix would shorten the complete source-to-unknot route."
                ),
            ),
        )
    output.commit()
    if output.execute("PRAGMA integrity_check").fetchone()[0] != "ok":
        raise RuntimeError("geodesic sidecar integrity check failed")
    output.execute("PRAGMA optimize")
    output.close()
    os.replace(temporary, args.output)

    counts = {distance: 0 for distance in (1, 2)}
    for row in derived.values():
        counts[row[0]] += 1
    print(
        json.dumps(
            {
                "schema": SCHEMA,
                "exact_sources": len(exact_sources),
                "exact_cc1_pairs": counts[1],
                "exact_cc2_pairs": counts[2],
                "skipped_unsupported_edge": skipped_unsupported_edge,
                "skipped_protected": skipped_protected,
                "skipped_nontrain": skipped_nontrain,
                "bytes": args.output.stat().st_size,
                "sha256": sha256(args.output),
            },
            indent=2,
            sort_keys=True,
        )
    )

    graph.close()
    identification.close()
    lower_bounds.close()
    pairs.close()
    hard.close()


if __name__ == "__main__":
    main()
