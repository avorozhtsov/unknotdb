#!/usr/bin/env python3
"""Build graph-pinned edge and identification provenance metadata."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sqlite3
from pathlib import Path


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def edge_key(row: tuple[object, ...]) -> bytes:
    digest = hashlib.sha256(b"UNKNOTDB_SEMANTIC_EDGE_V1\0")
    for value in row:
        if value is None:
            payload = b""
        elif isinstance(value, bytes):
            payload = value
        else:
            payload = str(value).encode()
        digest.update(len(payload).to_bytes(8, "big"))
        digest.update(payload)
    return digest.digest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--identifications", type=Path, required=True)
    parser.add_argument("--federation", type=Path, required=True)
    parser.add_argument("--release-version", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    for output in (args.output, args.report):
        if output.exists():
            raise FileExistsError(output)

    graph_sha256 = file_sha256(args.graph)
    identification_sha256 = file_sha256(args.identifications)
    federation_sha256 = file_sha256(args.federation)
    graph = sqlite3.connect(f"file:{args.graph}?mode=ro", uri=True)
    identifications = sqlite3.connect(f"file:{args.identifications}?mode=ro", uri=True)
    federation = sqlite3.connect(f"file:{args.federation}?mode=ro", uri=True)
    if (
        dict(identifications.execute("SELECT key,value FROM meta")).get(
            "graph_snapshot_sha256"
        )
        != graph_sha256
    ):
        raise ValueError("identification sidecar pins another graph")
    if (
        dict(federation.execute("SELECT key,value FROM meta")).get(
            "graph_snapshot_sha256"
        )
        != graph_sha256
    ):
        raise ValueError("federation sidecar pins another graph")

    temporary = args.output.with_name(f"{args.output.name}.tmp-{os.getpid()}")
    output = sqlite3.connect(temporary)
    try:
        output.executescript(
            """
            PRAGMA foreign_keys=ON;
            PRAGMA journal_mode=OFF;
            PRAGMA synchronous=OFF;
            CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;
            CREATE TABLE sources(
              source_id TEXT PRIMARY KEY,
              kind TEXT NOT NULL,
              title TEXT NOT NULL,
              source_uri TEXT NOT NULL,
              retrieved_at TEXT,
              content_sha256 TEXT CHECK(content_sha256 IS NULL OR length(content_sha256)=64),
              scope TEXT NOT NULL
            ) WITHOUT ROWID;
            CREATE TABLE edge_identities(
              semantic_edge_key BLOB PRIMARY KEY CHECK(length(semantic_edge_key)=32),
              graph_edge_id INTEGER NOT NULL UNIQUE,
              cc_cost INTEGER NOT NULL CHECK(cc_cost IN (0,1)),
              has_certificate INTEGER NOT NULL CHECK(has_certificate IN (0,1))
            ) WITHOUT ROWID;
            CREATE TABLE edge_provenance(
              semantic_edge_key BLOB NOT NULL,
              source_id TEXT NOT NULL,
              source_pointer TEXT NOT NULL,
              role TEXT NOT NULL,
              evidence_class TEXT NOT NULL,
              PRIMARY KEY(semantic_edge_key,source_id,source_pointer),
              FOREIGN KEY(semantic_edge_key) REFERENCES edge_identities(semantic_edge_key),
              FOREIGN KEY(source_id) REFERENCES sources(source_id)
            ) WITHOUT ROWID;
            CREATE TABLE mapping_provenance(
              representation_id TEXT NOT NULL,
              knot_id TEXT NOT NULL,
              evidence_class TEXT NOT NULL,
              evidence_kind TEXT NOT NULL,
              source_id TEXT NOT NULL,
              source_pointer TEXT NOT NULL,
              details_json TEXT NOT NULL,
              PRIMARY KEY(representation_id,knot_id,evidence_class,source_id,source_pointer),
              FOREIGN KEY(source_id) REFERENCES sources(source_id)
            ) WITHOUT ROWID;
            CREATE INDEX mapping_by_knot ON mapping_provenance(knot_id,evidence_class);
            CREATE TABLE run_provenance(
              run_id TEXT PRIMARY KEY,
              graph_source_generation TEXT NOT NULL,
              policy_model_id TEXT NOT NULL,
              preprocessing_adapter_version TEXT NOT NULL,
              validator_version TEXT NOT NULL
            ) WITHOUT ROWID;
            """
        )
        output.executemany(
            "INSERT INTO meta VALUES (?,?)",
            sorted(
                {
                    "schema": "unknotdb-provenance-sidecar-v1",
                    "release_version": args.release_version,
                    "graph_snapshot_sha256": graph_sha256,
                    "identification_sidecar_sha256": identification_sha256,
                    "federation_sidecar_sha256": federation_sha256,
                    "trust_boundary": "metadata-only-never-mutates-proof-values",
                    "edge_origin_policy": "known-external-links-only-no-inference",
                }.items()
            ),
        )
        output.executemany(
            "INSERT INTO sources VALUES (?,?,?,?,?,?,?)",
            (
                (
                    f"federation:{source_id}",
                    kind,
                    source_id,
                    uri,
                    retrieved_at,
                    sha256,
                    scope,
                )
                for source_id, kind, uri, retrieved_at, sha256, _format, scope in federation.execute(
                    "SELECT * FROM catalogue_sources ORDER BY source_id"
                )
            ),
        )
        graph_meta = dict(graph.execute("SELECT key,value FROM meta"))
        output.execute(
            "INSERT INTO sources VALUES (?,?,?,?,?,?,?)",
            (
                "proof-graph",
                "proof_graph",
                "Unknot DB proof graph snapshot",
                str(args.graph),
                None,
                graph_sha256,
                "replay-validated immutable programs and edges",
            ),
        )
        edge_keys: dict[int, bytes] = {}
        edge_rows = graph.execute(
            """
            SELECT e.edge_id,ks.rep_key,kt.rep_key,e.cc_cost,p.program_sha256,
                   e.program_anchor_x,e.program_anchor_y,e.certificate_id
            FROM edges e
            JOIN node_keys ks ON ks.node_id=e.source_node
            JOIN node_keys kt ON kt.node_id=e.target_node
            JOIN programs p USING(program_id)
            ORDER BY e.edge_id
            """
        )
        identities = []
        for edge_id, *semantic_parts in edge_rows:
            semantic = edge_key(tuple(semantic_parts))
            edge_keys[int(edge_id)] = semantic
            identities.append(
                (
                    semantic,
                    int(edge_id),
                    int(semantic_parts[2]),
                    semantic_parts[-1] is not None,
                )
            )
        output.executemany("INSERT INTO edge_identities VALUES (?,?,?,?)", identities)

        external_rows = list(
            federation.execute(
                """
                SELECT a.graph_edge_id,a.source_id,a.source_pointer,a.status
                FROM adjacency_claims a
                WHERE a.status='replay_verified' AND a.graph_edge_id IS NOT NULL
                ORDER BY a.graph_edge_id,a.source_id,a.source_pointer
                """
            )
        )
        output.executemany(
            "INSERT OR IGNORE INTO edge_provenance VALUES (?,?,?,?,?)",
            (
                (
                    edge_keys[int(edge_id)],
                    f"federation:{source_id}",
                    pointer,
                    "crossing-change witness association",
                    status,
                )
                for edge_id, source_id, pointer, status in external_rows
            ),
        )

        mapping_rows = list(
            identifications.execute(
                """
                SELECT m.representation_id,m.knot_id,m.evidence_class,
                       e.evidence_kind,e.source_path,e.source_sha256,
                       e.source_pointer,e.details_json
                FROM effective_representation_knot_map m
                JOIN identification_evidence e USING(evidence_id)
                ORDER BY m.representation_id,m.knot_id
                """
            )
        )
        mapping_sources = {}
        for (
            _rep,
            _knot,
            _class,
            _kind,
            path,
            sha256,
            _pointer,
            _details,
        ) in mapping_rows:
            source_id = (
                "mapping:"
                + hashlib.sha256((path + "\0" + sha256).encode()).hexdigest()[:24]
            )
            mapping_sources[(path, sha256)] = source_id
        output.executemany(
            "INSERT INTO sources VALUES (?,?,?,?,?,?,?)",
            (
                (
                    source_id,
                    "mapping_evidence",
                    path,
                    path,
                    None,
                    sha256,
                    "knot identification",
                )
                for (path, sha256), source_id in sorted(mapping_sources.items())
            ),
        )
        output.executemany(
            "INSERT INTO mapping_provenance VALUES (?,?,?,?,?,?,?)",
            (
                (
                    rep,
                    knot,
                    evidence_class,
                    evidence_kind,
                    mapping_sources[(path, sha256)],
                    pointer,
                    details,
                )
                for rep, knot, evidence_class, evidence_kind, path, sha256, pointer, details in mapping_rows
            ),
        )
        output.execute(
            "INSERT INTO run_provenance VALUES (?,?,?,?,?)",
            (
                args.release_version,
                graph_meta["source_generation"],
                graph_meta["policy_model_id"],
                graph_meta["policy_adapter_version"],
                graph_meta["validator_version"],
            ),
        )
        output.execute("PRAGMA optimize")
        integrity = output.execute("PRAGMA integrity_check").fetchone()[0]
        foreign_keys = output.execute("PRAGMA foreign_key_check").fetchall()
        if integrity != "ok" or foreign_keys:
            raise ValueError(
                f"provenance validation failed: {integrity}, {foreign_keys[:3]}"
            )
        counts = {
            table: output.execute(f"SELECT count(*) FROM {table}").fetchone()[0]
            for table in (
                "sources",
                "edge_identities",
                "edge_provenance",
                "mapping_provenance",
                "run_provenance",
            )
        }
        output.commit()
        output.close()
        os.replace(temporary, args.output)
    finally:
        graph.close()
        identifications.close()
        federation.close()
        if output:
            try:
                output.close()
            except sqlite3.Error:
                pass
        temporary.unlink(missing_ok=True)

    report = {
        "format": "unknotdb-provenance-report-v1",
        "release_version": args.release_version,
        "graph_snapshot_sha256": graph_sha256,
        "identification_sidecar_sha256": identification_sha256,
        "federation_sidecar_sha256": federation_sha256,
        "output": str(args.output),
        "output_sha256": file_sha256(args.output),
        "output_bytes": args.output.stat().st_size,
        "counts": counts,
        "known_external_edge_fraction": counts["edge_provenance"]
        / counts["edge_identities"],
        "validation": {"integrity_check": "ok", "foreign_key_check_rows": 0},
    }
    args.report.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps(report, sort_keys=True))


if __name__ == "__main__":
    main()
