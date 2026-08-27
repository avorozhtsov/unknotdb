#!/usr/bin/env python3
"""Build proof-external knot labels and invariant metadata for Unknot DB."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sqlite3
import sys
from pathlib import Path


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def audit_keys(paths: list[Path]) -> dict[str, str]:
    result: dict[str, str] = {}
    for path in paths:
        for line in path.read_text().splitlines():
            fields = line.split("\t")
            if len(fields) >= 5 and fields[0] == "audit" and fields[1] != "representation_id":
                key = fields[4]
                if len(key) == 64 and key != "-":
                    result[fields[1]] = key
            if len(fields) >= 3 and fields[0] == "representation" and fields[1] != "id":
                key = fields[2]
                if len(key) == 64 and key != "-":
                    result[fields[1]] = key
    return result


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--snapshot", type=Path, required=True)
    parser.add_argument("--corpus-json", type=Path, required=True)
    parser.add_argument("--expansion-json", type=Path)
    parser.add_argument("--knot-table", type=Path, required=True)
    parser.add_argument("--rf-src", type=Path, required=True)
    parser.add_argument("--audit", type=Path, action="append", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists() or args.report.exists():
        raise FileExistsError("output artifact already exists")

    corpus = json.loads(args.corpus_json.read_text())
    corpus_entries_by_id = {
        entry["representation_id"]: dict(entry) for entry in corpus["entries"]
    }
    if args.expansion_json:
        for entry in json.loads(args.expansion_json.read_text())["entries"]:
            identity = entry["representation_id"]
            if identity not in corpus_entries_by_id:
                corpus_entries_by_id[identity] = dict(entry)
                continue
            existing = corpus_entries_by_id[identity]
            refs = existing.setdefault("source_refs", [])
            for source in entry.get("source_refs", []):
                if source not in refs:
                    refs.append(source)
    corpus_entries = list(corpus_entries_by_id.values())
    table_document = json.loads(args.knot_table.read_text())
    table = table_document["knots"]
    stopping_keys = audit_keys(args.audit)

    graph = sqlite3.connect(f"file:{args.snapshot}?mode=ro", uri=True)
    graph_rows = {
        bytes(key).hex(): (int(node_id), int(u))
        for key, node_id, u in graph.execute(
            "SELECT k.rep_key,k.node_id,n.u_upper_bound FROM node_keys k JOIN nodes n USING(node_id)"
        )
    }
    graph.close()

    sys.path.insert(0, str(args.rf_src))
    from rf_knots.invariants import alexander_polynomial, to_pairs

    temp = args.output.with_name(args.output.name + f".tmp-{os.getpid()}")
    connection = sqlite3.connect(temp)
    connection.executescript(
        """
        PRAGMA journal_mode=OFF;
        PRAGMA synchronous=OFF;
        CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;
        CREATE TABLE representations(
            representation_id TEXT PRIMARY KEY,
            stopping_key TEXT,
            graph_node_id INTEGER,
            graph_u_upper INTEGER,
            status TEXT NOT NULL
        ) WITHOUT ROWID;
        CREATE TABLE identifications(
            representation_id TEXT NOT NULL,
            canonical_name TEXT NOT NULL,
            role TEXT NOT NULL,
            source_path TEXT NOT NULL,
            source_pointer TEXT NOT NULL,
            evidence_kind TEXT NOT NULL,
            PRIMARY KEY(representation_id,canonical_name,role,source_path,source_pointer)
        ) WITHOUT ROWID;
        CREATE INDEX identifications_by_name ON identifications(canonical_name);
        CREATE TABLE invariants(
            canonical_name TEXT PRIMARY KEY,
            crossings INTEGER NOT NULL,
            determinant INTEGER NOT NULL,
            alexander_json TEXT NOT NULL,
            jones_json TEXT NOT NULL,
            signature INTEGER,
            signature_status TEXT NOT NULL,
            signature_lower_bound INTEGER
        ) WITHOUT ROWID;
        CREATE TABLE known_u_claims(
            representation_id TEXT NOT NULL,
            source_id TEXT NOT NULL,
            claimed_u INTEGER NOT NULL,
            source_path TEXT NOT NULL,
            source_pointer TEXT NOT NULL,
            PRIMARY KEY(representation_id,source_id,source_path,source_pointer)
        ) WITHOUT ROWID;
        """
    )
    meta = {
        "schema": "unknotdb-identification-sidecar-v0",
        "snapshot_sha256": file_sha256(args.snapshot),
        "corpus_sha256": file_sha256(args.corpus_json),
        "knot_table_sha256": file_sha256(args.knot_table),
        "proof_status": "metadata-only-not-part-of-proof-graph",
        "signature_status": "not-computed-spherogram-unavailable",
    }
    if args.expansion_json:
        meta["expansion_sha256"] = file_sha256(args.expansion_json)
    connection.executemany("INSERT INTO meta VALUES (?,?)", sorted(meta.items()))

    identified_representations: set[str] = set()
    used_names: set[str] = set()
    connected = 0
    for entry in corpus_entries:
        representation_id = entry["representation_id"]
        key = stopping_keys.get(representation_id)
        graph_hit = graph_rows.get(key) if key else None
        if graph_hit:
            connected += 1
        status = "connected" if graph_hit else "no-attested-stopping-key"
        connection.execute(
            "INSERT INTO representations VALUES (?,?,?,?,?)",
            (
                representation_id,
                key,
                graph_hit[0] if graph_hit else None,
                graph_hit[1] if graph_hit else None,
                status,
            ),
        )
        for source in entry.get("source_refs", []):
            source_id = source.get("source_id")
            if source_id in table or source.get("lookup_name"):
                identified_representations.add(representation_id)
                if source_id in table:
                    used_names.add(source_id)
                connection.execute(
                    "INSERT OR IGNORE INTO identifications VALUES (?,?,?,?,?,?)",
                    (
                        representation_id,
                        source_id,
                        source.get("role", "unknown"),
                        source.get("path", ""),
                        source.get("pointer", ""),
                        (
                            "RF bundled KnotInfo/Spherogram catalogue mapping"
                            if source_id in table
                            else "Spherogram named-link lookup and braid conversion"
                        ),
                    ),
                )
            known_u = source.get("known_unknotting_number")
            if source_id is not None and isinstance(known_u, int):
                connection.execute(
                    "INSERT OR IGNORE INTO known_u_claims VALUES (?,?,?,?,?)",
                    (
                        representation_id,
                        source_id,
                        known_u,
                        source.get("path", ""),
                        source.get("pointer", ""),
                    ),
                )

    for index, name in enumerate(sorted(used_names), 1):
        entry = table[name]
        alexander = to_pairs(
            alexander_polynomial(tuple(entry["braid"]), int(entry["strands"]))
        )
        connection.execute(
            "INSERT INTO invariants VALUES (?,?,?,?,?,?,?,?)",
            (
                name,
                int(entry["crossings"]),
                int(entry["determinant"]),
                json.dumps(alexander, separators=(",", ":")),
                json.dumps(entry["jones"], separators=(",", ":")),
                None,
                "not-computed-spherogram-unavailable",
                None,
            ),
        )
        if index % 500 == 0:
            print(f"computed invariants {index}/{len(used_names)}", flush=True)

    connection.execute("PRAGMA optimize")
    integrity = connection.execute("PRAGMA integrity_check").fetchone()[0]
    if integrity != "ok":
        raise RuntimeError(f"sidecar integrity failure: {integrity}")
    connection.commit()
    connection.close()
    os.replace(temp, args.output)

    report = f"""# Knot identification and invariant sidecar

- Graph snapshot: `{args.snapshot}`
- Corpus representations: {len(corpus_entries):,}
- Representations mapped to current graph stopping keys: {connected:,}
- Representations with a bundled canonical knot name: {len(identified_representations):,}
- Distinct named knots with determinant, Alexander and Jones data: {len(used_names):,}
- Signature: not computed because Spherogram is unavailable in the existing RF environment.
- Sidecar: `{args.output}` ({args.output.stat().st_size:,} bytes)

This database is deliberately separate from the proof snapshot. Names, catalogue values and claimed tabular unknotting numbers are provenance-bearing metadata; they do not alter replay-validated `U_upper` or establish lower bounds. The Alexander polynomial is recomputed from each stored braid by RF Knots' exact invariant code. Determinant and Jones polynomial retain the bundled table provenance.
"""
    args.report.write_text(report)
    print(
        f"connected={connected} identified={len(identified_representations)} "
        f"named_knots={len(used_names)} bytes={args.output.stat().st_size}"
    )


if __name__ == "__main__":
    main()
