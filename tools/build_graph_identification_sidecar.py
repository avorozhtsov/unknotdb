#!/usr/bin/env python3
"""Build graph-wide knot-identifier postings with bounded candidate discovery."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import sqlite3
import sys
import time
from collections import defaultdict
from pathlib import Path
from typing import Any

SCHEMA = "unknotdb-graph-identification-v4"


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def canonical_json(value: Any) -> str:
    return json.dumps(value, separators=(",", ":"), sort_keys=True)


def evidence_id(payload: dict[str, Any]) -> bytes:
    return hashlib.sha256(
        b"UNKNOTDB_IDENTIFICATION_EVIDENCE_V1\0" + canonical_json(payload).encode()
    ).digest()


def add_evidence(
    db: sqlite3.Connection,
    evidence_class: str,
    evidence_kind: str,
    source_path: str,
    source_sha256: str,
    source_pointer: str,
    details: dict[str, Any],
) -> bytes:
    payload = {
        "class": evidence_class,
        "kind": evidence_kind,
        "source_path": source_path,
        "source_sha256": source_sha256,
        "source_pointer": source_pointer,
        "details": details,
    }
    identity = evidence_id(payload)
    db.execute(
        "INSERT OR IGNORE INTO identification_evidence VALUES (?,?,?,?,?,?,?)",
        (
            identity,
            evidence_class,
            evidence_kind,
            source_path,
            source_sha256,
            source_pointer,
            canonical_json(details),
        ),
    )
    return identity


def read_varint(data: bytes, cursor: int) -> tuple[int, int]:
    value = 0
    for shift in range(0, 64, 7):
        if cursor >= len(data):
            raise ValueError("truncated packed varint")
        byte = data[cursor]
        cursor += 1
        value |= (byte & 0x7F) << shift
        if byte & 0x80 == 0:
            return value, cursor
    raise ValueError("packed varint overflow")


def unpack_letter(code: int) -> int:
    generator = code // 2 + 1
    return generator if code % 2 == 0 else -generator


def decode_representation(data: bytes) -> tuple[int, bool, tuple[int, ...]]:
    if data.startswith(b"UKB0"):
        if len(data) < 12 or data[4] != 0:
            raise ValueError("invalid legacy braid representation")
        flags = data[5]
        strands = int.from_bytes(data[6:8], "little")
        length = int.from_bytes(data[8:12], "little")
        if len(data) != 12 + 2 * length:
            raise ValueError("legacy braid length mismatch")
        word = tuple(
            int.from_bytes(data[index : index + 2], "little", signed=True)
            for index in range(12, len(data), 2)
        )
        return strands, bool(flags & 1), word
    if not data or data[0] != 0xB1 or len(data) < 2:
        raise ValueError("invalid packed braid representation")
    flags = data[1]
    strands, cursor = read_varint(data, 2)
    length, cursor = read_varint(data, cursor)
    mode = flags & 6
    payload = data[cursor:]
    if mode == 2:
        if len(payload) != (length + 1) // 2:
            raise ValueError("packed nibble length mismatch")
        codes = [
            (payload[index // 2] >> (4 * (index % 2))) & 0xF for index in range(length)
        ]
    elif mode == 4:
        if len(payload) != length:
            raise ValueError("packed byte length mismatch")
        codes = list(payload)
    elif mode == 0:
        if len(payload) != 2 * length:
            raise ValueError("packed wide length mismatch")
        codes = [
            int.from_bytes(payload[index : index + 2], "little")
            for index in range(0, len(payload), 2)
        ]
    else:
        raise ValueError("conflicting packed braid width flags")
    return int(strands), bool(flags & 1), tuple(map(unpack_letter, codes))


class DisjointSet:
    def __init__(self, size: int) -> None:
        self.parent = list(range(size))
        self.weight = [1] * size

    def find(self, item: int) -> int:
        while self.parent[item] != item:
            self.parent[item] = self.parent[self.parent[item]]
            item = self.parent[item]
        return item

    def union(self, left: int, right: int) -> None:
        left = self.find(left)
        right = self.find(right)
        if left == right:
            return
        if self.weight[left] < self.weight[right]:
            left, right = right, left
        self.parent[right] = left
        self.weight[left] += self.weight[right]


def mirror_orbit_bundle(
    word: tuple[int, ...],
    strands: int,
    invariants: Any,
) -> tuple[str, str, str, str, bytes]:
    alexander = canonical_json(
        invariants.to_pairs(invariants.alexander_polynomial(word, strands))
    )
    jones_pairs = invariants.to_pairs(invariants.jones_polynomial(word, strands))
    jones = canonical_json(jones_pairs)
    mirror_jones = canonical_json(
        [[-exponent, coefficient] for exponent, coefficient in reversed(jones_pairs)]
    )
    signature = invariants.signature(word, strands)
    if signature is None:
        raise ValueError("signature unavailable")
    determinant = str(invariants.determinant(word, strands))
    bundle = {
        "determinant": determinant,
        "alexander": alexander,
        "jones_mirror_orbit": min(jones, mirror_jones),
        "absolute_signature": str(abs(int(signature))),
    }
    digest = hashlib.sha256(
        b"UNKNOTDB_GRAPH_INVARIANT_MIRROR_ORBIT_V0\0" + canonical_json(bundle).encode()
    ).digest()
    return (
        determinant,
        alexander,
        bundle["jones_mirror_orbit"],
        bundle["absolute_signature"],
        digest,
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--parent", required=True, type=Path)
    parser.add_argument("--graph", required=True, type=Path)
    parser.add_argument("--rf-src", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--max-invariant-nodes", type=int, default=6000)
    parser.add_argument("--max-candidates", type=int, default=1000)
    parser.add_argument("--deadline-seconds", type=int, default=10800)
    args = parser.parse_args()
    if args.output.exists() or args.report.exists():
        raise FileExistsError("output artifact already exists")
    started = time.monotonic()
    deadline = started + args.deadline_seconds
    parent_sha256 = file_sha256(args.parent)
    graph_sha256 = file_sha256(args.graph)

    temp = args.output.with_name(args.output.name + f".tmp-{os.getpid()}")
    shutil.copyfile(args.parent, temp)
    db = sqlite3.connect(temp)
    db.execute("PRAGMA foreign_keys=ON")
    parent_schema = db.execute("SELECT value FROM meta WHERE key='schema'").fetchone()
    if parent_schema != ("unknotdb-identification-maps-v3",):
        raise ValueError(f"unsupported parent schema: {parent_schema}")
    graph = sqlite3.connect(f"file:{args.graph}?mode=ro", uri=True)
    graph.row_factory = sqlite3.Row
    graph_meta = dict(graph.execute("SELECT key,value FROM meta"))
    if graph_meta.get("synthetic") != "0":
        raise ValueError("graph-wide identification requires a production snapshot")

    node_rows = list(
        graph.execute(
            """
            SELECT n.node_id,n.u_upper_bound,n.next_unknot_target,
                   n.next_unknot_cc_cost,nk.rep_key,r.encoding
            FROM nodes n JOIN node_keys nk USING(node_id)
            JOIN representations r USING(node_id) ORDER BY n.node_id
            """
        )
    )
    node_count = len(node_rows)
    if any(row["node_id"] != index for index, row in enumerate(node_rows)):
        raise ValueError("graph node ids are not dense")
    keys = [bytes(row["rep_key"]) for row in node_rows]
    key_to_node = {key: node for node, key in enumerate(keys)}

    sets = DisjointSet(node_count)
    zero_edges = 0
    for source, target in graph.execute(
        "SELECT source_node,target_node FROM edges WHERE cc_cost=0"
    ):
        sets.union(int(source), int(target))
        zero_edges += 1
    component_nodes: dict[int, list[int]] = defaultdict(list)
    for node in range(node_count):
        component_nodes[sets.find(node)].append(node)
    component_key: dict[int, bytes] = {
        root: min(keys[node] for node in nodes)
        for root, nodes in component_nodes.items()
    }

    db.executescript(
        """
        DROP VIEW effective_representation_knot_map;
        DROP VIEW unidentified_representations;
        CREATE TABLE graph_vertices(
            rep_key BLOB PRIMARY KEY CHECK(length(rep_key)=32),
            node_id INTEGER NOT NULL UNIQUE,
            encoding BLOB NOT NULL,
            u_upper INTEGER,
            cc0_component_key BLOB NOT NULL CHECK(length(cc0_component_key)=32)
        ) WITHOUT ROWID;
        CREATE INDEX graph_vertices_by_component
            ON graph_vertices(cc0_component_key,node_id);
        CREATE TABLE cc0_component_knot_seeds(
            cc0_component_key BLOB NOT NULL,
            knot_id TEXT NOT NULL,
            representation_id TEXT NOT NULL,
            evidence_class TEXT NOT NULL,
            PRIMARY KEY(cc0_component_key,knot_id,representation_id)
        ) WITHOUT ROWID;
        CREATE INDEX cc0_seeds_by_knot
            ON cc0_component_knot_seeds(knot_id,cc0_component_key);
        CREATE TABLE cc0_component_conflicts(
            cc0_component_key BLOB NOT NULL,
            knot_id TEXT NOT NULL,
            seed_count INTEGER NOT NULL,
            PRIMARY KEY(cc0_component_key,knot_id)
        ) WITHOUT ROWID;
        CREATE TABLE graph_vertex_knot_map(
            rep_key BLOB PRIMARY KEY CHECK(length(rep_key)=32),
            knot_id TEXT NOT NULL,
            mirror_bit INTEGER,
            evidence_class TEXT NOT NULL CHECK(evidence_class IN ('verified','attested')),
            mapping_status TEXT NOT NULL,
            evidence_id BLOB NOT NULL,
            FOREIGN KEY(rep_key) REFERENCES graph_vertices(rep_key),
            FOREIGN KEY(knot_id) REFERENCES knot_ids(knot_id),
            FOREIGN KEY(evidence_id) REFERENCES identification_evidence(evidence_id)
        ) WITHOUT ROWID;
        CREATE INDEX graph_vertex_postings
            ON graph_vertex_knot_map(knot_id,evidence_class,rep_key);
        CREATE TABLE graph_invariant_fingerprints(
            rep_key BLOB PRIMARY KEY CHECK(length(rep_key)=32),
            determinant TEXT NOT NULL,
            alexander_json TEXT NOT NULL,
            jones_mirror_orbit_json TEXT NOT NULL,
            absolute_signature TEXT NOT NULL,
            bundle_sha256 BLOB NOT NULL CHECK(length(bundle_sha256)=32),
            FOREIGN KEY(rep_key) REFERENCES graph_vertices(rep_key)
        ) WITHOUT ROWID;
        CREATE INDEX graph_invariants_by_bundle
            ON graph_invariant_fingerprints(bundle_sha256,rep_key);
        CREATE TABLE graph_equivalence_candidates(
            candidate_id BLOB PRIMARY KEY CHECK(length(candidate_id)=32),
            left_rep_key BLOB NOT NULL,
            right_rep_key BLOB NOT NULL,
            common_target_key BLOB NOT NULL,
            candidate_knot_id TEXT NOT NULL,
            invariant_bundle_sha256 BLOB NOT NULL,
            candidate_rank INTEGER NOT NULL,
            status TEXT NOT NULL CHECK(status IN ('pending','rejected','attested','verified')),
            evidence_id BLOB NOT NULL,
            FOREIGN KEY(left_rep_key) REFERENCES graph_vertices(rep_key),
            FOREIGN KEY(right_rep_key) REFERENCES graph_vertices(rep_key),
            FOREIGN KEY(common_target_key) REFERENCES graph_vertices(rep_key),
            FOREIGN KEY(candidate_knot_id) REFERENCES knot_ids(knot_id),
            FOREIGN KEY(evidence_id) REFERENCES identification_evidence(evidence_id)
        ) WITHOUT ROWID;
        CREATE INDEX graph_candidates_by_status
            ON graph_equivalence_candidates(status,candidate_rank,candidate_id);
        CREATE VIEW effective_representation_knot_map AS
            SELECT representation_id,knot_id,mirror_bit,'verified' AS evidence_class,
                   mapping_status,evidence_id
            FROM verified_representation_knot_map
            UNION ALL
            SELECT representation_id,knot_id,mirror_bit,'attested' AS evidence_class,
                   mapping_status,evidence_id
            FROM attested_representation_knot_map;
        CREATE VIEW unidentified_representations AS
            SELECT r.* FROM representations r
            WHERE NOT EXISTS(
                SELECT 1 FROM effective_representation_knot_map m
                WHERE m.representation_id=r.representation_id
            );
        CREATE VIEW knot_representation_postings AS
            SELECT knot_id,'source' AS representation_namespace,
                   representation_id AS representation_ref,NULL AS rep_key,
                   NULL AS graph_node_id,mirror_bit,evidence_class,mapping_status
            FROM effective_representation_knot_map
            UNION ALL
            SELECT m.knot_id,'graph' AS representation_namespace,
                   lower(hex(m.rep_key)) AS representation_ref,m.rep_key,
                   v.node_id,m.mirror_bit,m.evidence_class,m.mapping_status
            FROM graph_vertex_knot_map m JOIN graph_vertices v USING(rep_key);
        """
    )
    db.execute("UPDATE meta SET value=? WHERE key='schema'", (SCHEMA,))
    db.executemany(
        "INSERT OR REPLACE INTO meta VALUES (?,?)",
        (
            ("parent_identification_sha256", parent_sha256),
            ("graph_snapshot_sha256", graph_sha256),
            ("graph_validator_version", graph_meta["validator_version"]),
            (
                "cc0_equivalence_policy",
                "undirected-closure-of-replay-validated-cc0-edges-v0",
            ),
            (
                "candidate_policy",
                "same-optimal-one-cc-target-and-exact-mirror-orbit-invariant-bundle-v0",
            ),
        ),
    )
    db.executemany(
        "INSERT INTO graph_vertices VALUES (?,?,?,?,?)",
        (
            (
                keys[node],
                node,
                bytes(row["encoding"]),
                row["u_upper_bound"],
                component_key[sets.find(node)],
            )
            for node, row in enumerate(node_rows)
        ),
    )

    # Remap source representations to the latest graph by stable canonical key.
    for identity, stopping_key in db.execute(
        "SELECT representation_id,stopping_key FROM representations"
    ).fetchall():
        node = (
            key_to_node.get(bytes(stopping_key)) if stopping_key is not None else None
        )
        if node is None:
            db.execute(
                "UPDATE representations SET graph_node_id=NULL,graph_u_upper=NULL,graph_status='miss-current-graph' WHERE representation_id=?",
                (identity,),
            )
        else:
            db.execute(
                "UPDATE representations SET graph_node_id=?,graph_u_upper=?,graph_status='hit-current-graph' WHERE representation_id=?",
                (node, node_rows[node]["u_upper_bound"], identity),
            )

    labels: dict[int, dict[str, list[tuple[str, str]]]] = defaultdict(
        lambda: defaultdict(list)
    )
    for identity, knot_id, evidence_class, stopping_key in db.execute(
        """
        SELECT m.representation_id,m.knot_id,m.evidence_class,r.stopping_key
        FROM effective_representation_knot_map m
        JOIN representations r USING(representation_id)
        WHERE r.stopping_key IS NOT NULL
        """
    ):
        node = key_to_node.get(bytes(stopping_key))
        if node is None:
            continue
        root = sets.find(node)
        labels[root][str(knot_id)].append((str(identity), str(evidence_class)))
        db.execute(
            "INSERT OR IGNORE INTO cc0_component_knot_seeds VALUES (?,?,?,?)",
            (component_key[root], knot_id, identity, evidence_class),
        )

    conflicts = 0
    conflicting_roots: set[int] = set()
    mapped_components = 0
    mapped_nodes = 0
    for root, knot_labels in sorted(
        labels.items(), key=lambda item: component_key[item[0]]
    ):
        if len(knot_labels) != 1:
            conflicts += 1
            conflicting_roots.add(root)
            for knot_id, seeds in knot_labels.items():
                db.execute(
                    "INSERT INTO cc0_component_conflicts VALUES (?,?,?)",
                    (component_key[root], knot_id, len(seeds)),
                )
            continue
        knot_id, seeds = next(iter(knot_labels.items()))
        evidence_class = (
            "verified" if any(kind == "verified" for _, kind in seeds) else "attested"
        )
        evidence = add_evidence(
            db,
            evidence_class,
            "replay-validated-cc0-component-propagation",
            str(args.graph),
            graph_sha256,
            f"cc0_component={component_key[root].hex()}",
            {
                "knot_id": knot_id,
                "component_nodes": len(component_nodes[root]),
                "seed_representations": [identity for identity, _ in seeds],
                "zero_cc_equivalence": True,
            },
        )
        for node in component_nodes[root]:
            db.execute(
                "INSERT INTO graph_vertex_knot_map VALUES (?,?,?,?,?,?)",
                (
                    keys[node],
                    knot_id,
                    None,
                    evidence_class,
                    "verified-cc0-equivalent"
                    if evidence_class == "verified"
                    else "attested-seed-cc0-equivalent",
                    evidence,
                ),
            )
        mapped_components += 1
        mapped_nodes += len(component_nodes[root])

    # Select exact common-target, optimal-one-CC sibling cohorts.
    by_target: dict[int, set[int]] = defaultdict(set)
    for node, row in enumerate(node_rows):
        if row["next_unknot_cc_cost"] == 1 and row["next_unknot_target"] is not None:
            by_target[int(row["next_unknot_target"])].add(sets.find(node))
    mapped_root: dict[int, str] = {}
    for root, knot_labels in labels.items():
        if len(knot_labels) == 1 and root not in conflicting_roots:
            mapped_root[root] = next(iter(knot_labels))
    mixed_targets = {
        target: roots
        for target, roots in by_target.items()
        if any(root in mapped_root for root in roots)
        and any(root not in mapped_root for root in roots)
    }
    representative = {
        root: min(
            component_nodes[root],
            key=lambda node: (
                len(decode_representation(bytes(node_rows[node]["encoding"]))[2]),
                decode_representation(bytes(node_rows[node]["encoding"]))[0],
                keys[node],
            ),
        )
        for roots in mixed_targets.values()
        for root in roots
    }
    needed_roots = sorted(
        representative,
        key=lambda root: (
            root not in mapped_root,
            len(
                decode_representation(
                    bytes(node_rows[representative[root]]["encoding"])
                )[2]
            ),
            keys[representative[root]],
        ),
    )[: args.max_invariant_nodes]
    sys.path.insert(0, str(args.rf_src))
    from rf_knots import invariants  # type: ignore[import-not-found]

    bundle_by_root: dict[int, bytes] = {}
    invariant_failures = 0
    for index, root in enumerate(needed_roots, 1):
        if time.monotonic() >= deadline:
            break
        node = representative[root]
        strands, cyclic, word = decode_representation(
            bytes(node_rows[node]["encoding"])
        )
        if cyclic:
            invariant_failures += 1
            continue
        try:
            values = mirror_orbit_bundle(word, strands, invariants)
        except Exception:  # noqa: BLE001 - one bad external invariant must not abort the batch
            invariant_failures += 1
            continue
        db.execute(
            "INSERT INTO graph_invariant_fingerprints VALUES (?,?,?,?,?,?)",
            (keys[node], *values),
        )
        bundle_by_root[root] = values[-1]
        if index % 500 == 0:
            print(f"graph-invariants {index}/{len(needed_roots)}", flush=True)

    candidates: dict[tuple[int, str], tuple[int, int, bytes]] = {}
    for target, roots in mixed_targets.items():
        labeled_by_bundle: dict[bytes, list[int]] = defaultdict(list)
        for root in roots:
            if root in mapped_root and root in bundle_by_root:
                labeled_by_bundle[bundle_by_root[root]].append(root)
        for left in roots:
            if left in mapped_root or left not in bundle_by_root:
                continue
            for right in labeled_by_bundle.get(bundle_by_root[left], []):
                knot_id = mapped_root[right]
                current = candidates.get((left, knot_id))
                proposed = (target, right, bundle_by_root[left])
                if current is None or (
                    len(
                        decode_representation(
                            bytes(node_rows[representative[right]]["encoding"])
                        )[2]
                    ),
                    keys[representative[right]],
                ) < (
                    len(
                        decode_representation(
                            bytes(node_rows[representative[current[1]]]["encoding"])
                        )[2]
                    ),
                    keys[representative[current[1]]],
                ):
                    candidates[(left, knot_id)] = proposed
    ordered_candidates = sorted(
        candidates.items(),
        key=lambda item: (
            len(
                decode_representation(
                    bytes(node_rows[representative[item[0][0]]]["encoding"])
                )[2]
            ),
            decode_representation(
                bytes(node_rows[representative[item[0][0]]]["encoding"])
            )[0],
            item[0][1],
            keys[representative[item[0][0]]],
        ),
    )[: args.max_candidates]
    for rank, ((left_root, knot_id), (target, right_root, bundle)) in enumerate(
        ordered_candidates, 1
    ):
        left = representative[left_root]
        right = representative[right_root]
        payload = {
            "left": keys[left].hex(),
            "right": keys[right].hex(),
            "target": keys[target].hex(),
            "knot_id": knot_id,
            "bundle": bundle.hex(),
        }
        candidate_identity = hashlib.sha256(
            b"UNKNOTDB_GRAPH_EQUIVALENCE_CANDIDATE_V0\0"
            + canonical_json(payload).encode()
        ).digest()
        evidence = add_evidence(
            db,
            "candidate",
            "same-optimal-one-cc-target-and-exact-invariants",
            str(args.graph),
            graph_sha256,
            f"candidate_id={candidate_identity.hex()}",
            payload,
        )
        db.execute(
            "INSERT INTO graph_equivalence_candidates VALUES (?,?,?,?,?,?,?,?,?)",
            (
                candidate_identity,
                keys[left],
                keys[right],
                keys[target],
                knot_id,
                bundle,
                rank,
                "pending",
                evidence,
            ),
        )

    counts = {
        "source_representations": db.execute(
            "SELECT count(*) FROM representations"
        ).fetchone()[0],
        "source_effective": db.execute(
            "SELECT count(*) FROM effective_representation_knot_map"
        ).fetchone()[0],
        "source_current_graph_hits": db.execute(
            "SELECT count(*) FROM representations WHERE graph_node_id IS NOT NULL"
        ).fetchone()[0],
        "graph_vertices": node_count,
        "zero_cc_edges": zero_edges,
        "cc0_components": len(component_nodes),
        "mapped_components": mapped_components,
        "mapped_graph_vertices": mapped_nodes,
        "component_conflicts": conflicts,
        "invariant_nodes": len(bundle_by_root),
        "invariant_failures": invariant_failures,
        "candidate_rows": len(ordered_candidates),
    }
    db.executemany(
        "INSERT OR REPLACE INTO meta VALUES (?,?)",
        ((f"count_{key}", str(value)) for key, value in counts.items()),
    )
    db.execute("PRAGMA optimize")
    integrity = db.execute("PRAGMA integrity_check").fetchone()[0]
    foreign_keys = db.execute("PRAGMA foreign_key_check").fetchall()
    if integrity != "ok" or foreign_keys:
        raise RuntimeError(f"integrity={integrity} foreign_keys={foreign_keys[:3]}")
    db.commit()
    db.close()
    graph.close()
    os.replace(temp, args.output)
    output_sha256 = file_sha256(args.output)
    elapsed = time.monotonic() - started
    args.report.write_text(
        "# Graph-wide knot identification sidecar v4\n\n"
        f"- Parent: `{args.parent}` (SHA-256 `{parent_sha256}`)\n"
        f"- Graph: `{args.graph}` (SHA-256 `{graph_sha256}`)\n"
        f"- Output: `{args.output}` ({args.output.stat().st_size:,} bytes; SHA-256 `{output_sha256}`)\n"
        f"- Elapsed: {elapsed:.1f} seconds\n"
        + "\n".join(
            f"- {key.replace('_', ' ').title()}: {value:,}"
            for key, value in counts.items()
        )
        + "\n- SQLite integrity and foreign keys: `ok`\n\n"
        "A CC=0 component is labelled only when all source seeds agree on one knot ID. "
        "Same-target/invariant rows remain candidates until an external equivalence checker promotes them.\n"
    )
    print(
        canonical_json({**counts, "elapsed_seconds": elapsed, "sha256": output_sha256})
    )


if __name__ == "__main__":
    main()
