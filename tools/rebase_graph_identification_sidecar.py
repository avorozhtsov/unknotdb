#!/usr/bin/env python3
"""Rebase graph identification postings onto a key-superset proof snapshot."""

from __future__ import annotations

import argparse
import hashlib
import os
import shutil
import sqlite3
from collections import defaultdict
from pathlib import Path


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


class DisjointSet:
    def __init__(self, size: int) -> None:
        self.parent = list(range(size))

    def find(self, item: int) -> int:
        while self.parent[item] != item:
            self.parent[item] = self.parent[self.parent[item]]
            item = self.parent[item]
        return item

    def union(self, left: int, right: int) -> None:
        left = self.find(left)
        right = self.find(right)
        if left != right:
            self.parent[max(left, right)] = min(left, right)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        raise FileExistsError(args.output)

    graph_sha256 = file_sha256(args.graph)
    graph = sqlite3.connect(f"file:{args.graph}?mode=ro", uri=True)
    graph_meta = dict(graph.execute("SELECT key,value FROM meta"))
    nodes = list(
        graph.execute(
            """
            SELECT n.node_id,k.rep_key,r.encoding,n.u_upper_bound
            FROM nodes n JOIN node_keys k USING(node_id)
            JOIN representations r USING(node_id)
            ORDER BY n.node_id
            """
        )
    )
    if any(node_id != index for index, (node_id, *_rest) in enumerate(nodes)):
        raise ValueError("graph node IDs are not dense")
    sets = DisjointSet(len(nodes))
    for source, target in graph.execute(
        "SELECT source_node,target_node FROM edges WHERE cc_cost=0"
    ):
        sets.union(int(source), int(target))
    component_nodes: dict[int, list[int]] = defaultdict(list)
    for node_id in range(len(nodes)):
        component_nodes[sets.find(node_id)].append(node_id)
    component_key = {
        root: min(bytes(nodes[node_id][1]) for node_id in members)
        for root, members in component_nodes.items()
    }
    key_to_node = {bytes(key): int(node_id) for node_id, key, _encoding, _u in nodes}

    temporary = args.output.with_name(f"{args.output.name}.tmp-{os.getpid()}")
    shutil.copyfile(args.input, temporary)
    sidecar = sqlite3.connect(temporary)
    try:
        old_graph_seeds = list(
            sidecar.execute(
                """
                SELECT rep_key,knot_id,evidence_class,mapping_status,evidence_id
                FROM graph_vertex_knot_map ORDER BY rep_key
                """
            )
        )
        source_seeds = list(
            sidecar.execute(
                """
                SELECT r.representation_id,r.stopping_key,m.knot_id,
                       m.evidence_class,m.mapping_status,m.evidence_id
                FROM representations r
                JOIN effective_representation_knot_map m USING(representation_id)
                WHERE r.stopping_key IS NOT NULL
                ORDER BY r.representation_id
                """
            )
        )
        old_keys = {
            bytes(key)
            for (key,) in sidecar.execute("SELECT rep_key FROM graph_vertices")
        }
        if not old_keys.issubset(key_to_node):
            raise ValueError("new graph is not a key superset of identification input")

        # Snapshot publication may densely reorder node IDs. Move the old IDs
        # out of the nonnegative target domain before assigning the new dense
        # ordering, while all graph references remain keyed by rep_key.
        sidecar.execute("UPDATE graph_vertices SET node_id=-(node_id+1)")
        for node_id, key, encoding, u_upper in nodes:
            key = bytes(key)
            root = sets.find(int(node_id))
            sidecar.execute(
                """
                INSERT INTO graph_vertices(rep_key,node_id,encoding,u_upper,cc0_component_key)
                VALUES (?,?,?,?,?)
                ON CONFLICT(rep_key) DO UPDATE SET
                  node_id=excluded.node_id,encoding=excluded.encoding,
                  u_upper=excluded.u_upper,cc0_component_key=excluded.cc0_component_key
                """,
                (key, node_id, bytes(encoding), u_upper, component_key[root]),
            )
        for identity, stopping_key in sidecar.execute(
            "SELECT representation_id,stopping_key FROM representations"
        ).fetchall():
            node_id = key_to_node.get(bytes(stopping_key)) if stopping_key else None
            if node_id is None:
                sidecar.execute(
                    """
                    UPDATE representations SET graph_node_id=NULL,graph_u_upper=NULL,
                      graph_status='miss-current-graph' WHERE representation_id=?
                    """,
                    (identity,),
                )
            else:
                sidecar.execute(
                    """
                    UPDATE representations SET graph_node_id=?,graph_u_upper=?,
                      graph_status='hit-current-graph' WHERE representation_id=?
                    """,
                    (node_id, nodes[node_id][3], identity),
                )

        seeds: dict[int, list[tuple[str, str, str, bytes, str]]] = defaultdict(list)
        for key, knot, evidence_class, status, evidence_id in old_graph_seeds:
            node_id = key_to_node[bytes(key)]
            seeds[sets.find(node_id)].append(
                (
                    str(knot),
                    str(evidence_class),
                    str(status),
                    bytes(evidence_id),
                    f"graph:{bytes(key).hex()}",
                )
            )
        for identity, key, knot, evidence_class, status, evidence_id in source_seeds:
            node_id = key_to_node.get(bytes(key))
            if node_id is not None:
                seeds[sets.find(node_id)].append(
                    (
                        str(knot),
                        str(evidence_class),
                        str(status),
                        bytes(evidence_id),
                        str(identity),
                    )
                )

        sidecar.execute("DELETE FROM graph_vertex_knot_map")
        sidecar.execute("DELETE FROM cc0_component_knot_seeds")
        sidecar.execute("DELETE FROM cc0_component_conflicts")
        evidence_rank = {"verified": 0, "attested": 1}
        mapped = 0
        conflicts = 0
        seed_rows = set()
        for root, entries in seeds.items():
            for knot, evidence_class, _status, _evidence_id, identity in entries:
                seed_rows.add((component_key[root], knot, identity, evidence_class))
            by_knot: dict[str, list[tuple[str, str, bytes, str]]] = defaultdict(list)
            for knot, evidence_class, status, evidence_id, identity in entries:
                by_knot[knot].append((evidence_class, status, evidence_id, identity))
            if len(by_knot) != 1:
                conflicts += 1
                sidecar.executemany(
                    "INSERT INTO cc0_component_conflicts VALUES (?,?,?)",
                    (
                        (component_key[root], knot, len(knot_entries))
                        for knot, knot_entries in sorted(by_knot.items())
                    ),
                )
                continue
            knot, knot_entries = next(iter(by_knot.items()))
            chosen = min(
                knot_entries,
                key=lambda row: (evidence_rank[row[0]], row[3], row[2]),
            )
            evidence_class, _status, evidence_id, _identity = chosen
            status = f"{evidence_class}-cc0-equivalent"
            for node_id in component_nodes[root]:
                key = bytes(nodes[node_id][1])
                sidecar.execute(
                    "INSERT INTO graph_vertex_knot_map VALUES (?,?,?,?,?,?)",
                    (key, knot, None, evidence_class, status, evidence_id),
                )
                mapped += 1
        sidecar.executemany(
            "INSERT INTO cc0_component_knot_seeds VALUES (?,?,?,?)",
            sorted(seed_rows),
        )
        sidecar.executemany(
            "INSERT OR REPLACE INTO meta VALUES (?,?)",
            (
                ("graph_snapshot_sha256", graph_sha256),
                ("graph_validator_version", graph_meta["validator_version"]),
                ("rebase_parent_identification_sha256", file_sha256(args.input)),
                ("count_graph_vertices", str(len(nodes))),
                (
                    "count_zero_cc_edges",
                    str(
                        sum(
                            1
                            for _ in graph.execute(
                                "SELECT 1 FROM edges WHERE cc_cost=0"
                            )
                        )
                    ),
                ),
                ("count_cc0_components", str(len(component_nodes))),
                ("count_mapped_graph_vertices", str(mapped)),
                ("count_component_conflicts", str(conflicts)),
            ),
        )
        sidecar.execute("PRAGMA optimize")
        integrity = sidecar.execute("PRAGMA integrity_check").fetchone()[0]
        foreign_keys = sidecar.execute("PRAGMA foreign_key_check").fetchall()
        if integrity != "ok" or foreign_keys:
            raise ValueError(
                f"rebase validation failed: {integrity}, {foreign_keys[:3]}"
            )
        sidecar.commit()
        sidecar.close()
        os.replace(temporary, args.output)
    finally:
        graph.close()
        try:
            sidecar.close()
        except sqlite3.Error:
            pass
        temporary.unlink(missing_ok=True)
    print(
        f"output={args.output} graph_sha256={graph_sha256} nodes={len(nodes)} "
        f"components={len(component_nodes)} mapped={mapped} conflicts={conflicts}"
    )


if __name__ == "__main__":
    main()
