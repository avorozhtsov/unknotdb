#!/usr/bin/env python3
"""Build tier-separated representation-to-knot identification maps."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sqlite3
import sys
from pathlib import Path
from typing import Any

SCHEMA = "unknotdb-identification-maps-v3"

LADDER_ATTESTATIONS = {
    "R(3,10)#0": "8_21",
    "R(5,14)#0": "3_1#5_2",
    "R(3,24)#0": "12n_570",
    "R(5,16)#0": "8_20",
    "T(3,5)": "10_124",
    "R(3,14)#0": "7_3",
    "R(3,18)#0": "7_5",
    "R(3,16)#0": "12n_647",
    "R(3,20)#0": "10_104",
    "R(5,10)#0": "3_1",
    "R(5,12)#0": "3_1#4_1",
    "R(5,18)#0": "3_1",
    "R(5,20)#0": "3_1#3_1#3_1",
    "R(3,12)#0": "6_3",
    "R(3,22)#0": "0_1",
    "unknot": "0_1",
}


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


def load_representations(paths: list[Path]) -> dict[str, dict[str, Any]]:
    result: dict[str, dict[str, Any]] = {}
    for path in paths:
        for entry in json.loads(path.read_text())["entries"]:
            identity = entry["representation_id"]
            previous = result.setdefault(identity, dict(entry))
            if (previous["word"], previous["strands"]) != (
                entry["word"],
                entry["strands"],
            ):
                raise ValueError(f"conflicting braid payload for {identity}")
            refs = previous.setdefault("source_refs", [])
            for source in entry.get("source_refs", []):
                if source not in refs:
                    refs.append(source)
    return result


def source_ids(entry: dict[str, Any]) -> set[str]:
    return {
        str(source["source_id"])
        for source in entry.get("source_refs", [])
        if source.get("source_id") is not None
    }


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


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--lookup-maps", type=Path, required=True)
    parser.add_argument("--source-identification", type=Path, required=True)
    parser.add_argument("--graph-snapshot", type=Path, required=True)
    parser.add_argument("--corpus-json", type=Path, action="append", required=True)
    parser.add_argument("--rf-root", type=Path, required=True)
    parser.add_argument("--rf-src", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists() or args.report.exists():
        raise FileExistsError("output artifact already exists")

    representations = load_representations(args.corpus_json)
    lookup = sqlite3.connect(f"file:{args.lookup_maps}?mode=ro", uri=True)
    lookup_meta = dict(lookup.execute("SELECT key,value FROM meta"))
    graph_sha256 = file_sha256(args.graph_snapshot)
    if lookup_meta["graph_snapshot_sha256"] != graph_sha256:
        raise ValueError("lookup maps and graph snapshot differ")
    graph_rows = {
        identity: (key, node_id, u_upper, status)
        for identity, key, node_id, u_upper, status in lookup.execute(
            "SELECT representation_id,stopping_key,graph_node_id,graph_u_upper,status "
            "FROM representation_graph_map"
        )
    }
    inherited = {
        identity: (knot_id, evidence_count, status)
        for identity, knot_id, evidence_count, status in lookup.execute(
            "SELECT representation_id,knot_id,evidence_count,mapping_status "
            "FROM representation_knot_map"
        )
    }
    vertex_names = dict(lookup.execute("SELECT stopping_key,knot_id FROM graph_vertex_knot_map"))
    known_knot_ids = {
        knot_id: (canonical_name, scheme)
        for knot_id, canonical_name, scheme in lookup.execute(
            "SELECT knot_id,canonical_name,identifier_scheme FROM knot_ids"
        )
    }
    known_invariants: list[tuple[str, dict[str, str]]] = []
    for knot_id in known_knot_ids:
        values = dict(
            lookup.execute(
                "SELECT invariant_id,value_text FROM knot_invariant_values WHERE knot_id=?",
                (knot_id,),
            )
        )
        if all(name in values for name in ("determinant", "alexander", "jones", "signature")):
            known_invariants.append((knot_id, values))
    lookup.close()

    source_identification_sha256 = file_sha256(args.source_identification)
    source_identification = sqlite3.connect(
        f"file:{args.source_identification}?mode=ro", uri=True
    )
    inherited_sources: dict[str, list[tuple[str, str, str, str]]] = {}
    for identity, name, path, pointer, kind in source_identification.execute(
        "SELECT representation_id,canonical_name,source_path,source_pointer,evidence_kind "
        "FROM identifications ORDER BY representation_id,source_path,source_pointer"
    ):
        inherited_sources.setdefault(identity, []).append((name, path, pointer, kind))
    source_identification.close()

    dkt_path = args.rf_root / "benchmarks/dkt2026-table1-authors-pd-braids-v1.json"
    dkt_sha256 = file_sha256(dkt_path)
    dkt_document = json.loads(dkt_path.read_text())
    dkt = {entry["instance_id"]: entry for entry in dkt_document["instances"]}
    ladder_path = args.rf_root / "docs/rungs-invariants.md"
    ladder_sha256 = file_sha256(ladder_path)

    sys.path.insert(0, str(args.rf_src))
    import spherogram  # type: ignore[import-not-found]
    from rf_knots.invariants import (  # type: ignore[import-not-found]
        alexander_polynomial,
        determinant,
        jones_polynomial,
        signature,
        to_pairs,
    )

    temp = args.output.with_name(args.output.name + f".tmp-{os.getpid()}")
    db = sqlite3.connect(temp)
    db.execute("PRAGMA foreign_keys=ON")
    db.executescript(
        """
        PRAGMA journal_mode=OFF;
        PRAGMA synchronous=OFF;
        CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;
        CREATE TABLE representations(
            representation_id TEXT PRIMARY KEY,
            stopping_key BLOB,
            graph_node_id INTEGER,
            graph_u_upper INTEGER,
            graph_status TEXT NOT NULL
        ) WITHOUT ROWID;
        CREATE TABLE knot_ids(
            knot_id TEXT PRIMARY KEY,
            canonical_name TEXT NOT NULL UNIQUE,
            identifier_scheme TEXT NOT NULL
        ) WITHOUT ROWID;
        CREATE TABLE identification_evidence(
            evidence_id BLOB PRIMARY KEY CHECK(length(evidence_id)=32),
            evidence_class TEXT NOT NULL CHECK(evidence_class IN ('verified','attested','candidate')),
            evidence_kind TEXT NOT NULL,
            source_path TEXT NOT NULL,
            source_sha256 TEXT NOT NULL,
            source_pointer TEXT NOT NULL,
            details_json TEXT NOT NULL
        ) WITHOUT ROWID;
        CREATE TABLE verified_representation_knot_map(
            representation_id TEXT PRIMARY KEY,
            knot_id TEXT NOT NULL,
            mirror_bit INTEGER NOT NULL CHECK(mirror_bit IN (0,1)),
            mapping_status TEXT NOT NULL,
            evidence_id BLOB NOT NULL,
            FOREIGN KEY(representation_id) REFERENCES representations(representation_id),
            FOREIGN KEY(knot_id) REFERENCES knot_ids(knot_id),
            FOREIGN KEY(evidence_id) REFERENCES identification_evidence(evidence_id)
        ) WITHOUT ROWID;
        CREATE INDEX verified_by_knot ON verified_representation_knot_map(knot_id);
        CREATE TABLE attested_representation_knot_map(
            representation_id TEXT PRIMARY KEY,
            knot_id TEXT NOT NULL,
            mirror_bit INTEGER,
            mapping_status TEXT NOT NULL,
            evidence_id BLOB NOT NULL,
            FOREIGN KEY(representation_id) REFERENCES representations(representation_id),
            FOREIGN KEY(knot_id) REFERENCES knot_ids(knot_id),
            FOREIGN KEY(evidence_id) REFERENCES identification_evidence(evidence_id)
        ) WITHOUT ROWID;
        CREATE INDEX attested_by_knot ON attested_representation_knot_map(knot_id);
        CREATE TABLE representation_invariant_fingerprints(
            representation_id TEXT PRIMARY KEY,
            determinant TEXT NOT NULL,
            alexander_json TEXT NOT NULL,
            jones_json TEXT NOT NULL,
            signature TEXT NOT NULL,
            bundle_sha256 BLOB NOT NULL CHECK(length(bundle_sha256)=32),
            FOREIGN KEY(representation_id) REFERENCES representations(representation_id)
        ) WITHOUT ROWID;
        CREATE TABLE representation_knot_candidates(
            representation_id TEXT NOT NULL,
            candidate_knot_id TEXT NOT NULL,
            mirror_bit INTEGER NOT NULL CHECK(mirror_bit IN (0,1)),
            candidate_rank INTEGER NOT NULL,
            evidence_id BLOB NOT NULL,
            PRIMARY KEY(representation_id,candidate_knot_id),
            FOREIGN KEY(representation_id) REFERENCES representations(representation_id),
            FOREIGN KEY(candidate_knot_id) REFERENCES knot_ids(knot_id),
            FOREIGN KEY(evidence_id) REFERENCES identification_evidence(evidence_id)
        ) WITHOUT ROWID;
        CREATE INDEX candidates_by_knot
            ON representation_knot_candidates(candidate_knot_id,representation_id);
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
        """
    )
    metadata = {
        "schema": SCHEMA,
        "lookup_maps_sha256": file_sha256(args.lookup_maps),
        "source_identification_sha256": source_identification_sha256,
        "graph_snapshot_sha256": graph_sha256,
        "dkt_manifest_sha256": dkt_sha256,
        "ladder_report_sha256": ladder_sha256,
        "candidate_policy": "never-promote-without-new-verified-or-attested-evidence",
    }
    for index, path in enumerate(args.corpus_json):
        metadata[f"corpus_{index}_sha256"] = file_sha256(path)
    db.executemany("INSERT INTO meta VALUES (?,?)", sorted(metadata.items()))
    db.executemany(
        "INSERT INTO representations VALUES (?,?,?,?,?)",
        [
            (identity, key, node_id, u_upper, status)
            for identity, (key, node_id, u_upper, status) in sorted(graph_rows.items())
        ],
    )
    db.executemany(
        "INSERT INTO knot_ids VALUES (?,?,?)",
        [(knot_id, *values) for knot_id, values in sorted(known_knot_ids.items())],
    )

    def ensure_knot(name: str) -> str:
        knot_id = f"knot:{name}"
        if knot_id not in known_knot_ids:
            scheme = "prime-sum-v1" if "#" in name else "canonical-catalogue-name-v1"
            db.execute("INSERT OR IGNORE INTO knot_ids VALUES (?,?,?)", (knot_id, name, scheme))
            known_knot_ids[knot_id] = (name, scheme)
        return knot_id

    assigned: set[str] = set()
    for identity, (knot_id, evidence_count, inherited_status) in sorted(inherited.items()):
        sources = inherited_sources.get(identity, [])
        details = {
            "parent_mapping_status": inherited_status,
            "parent_evidence_count": evidence_count,
            "sources": sources,
        }
        evidence = add_evidence(
            db,
            "attested",
            "inherited-provenance-bundle",
            str(args.source_identification),
            source_identification_sha256,
            f"representation_id={identity}",
            details,
        )
        db.execute(
            "INSERT INTO attested_representation_knot_map VALUES (?,?,?,?,?)",
            (identity, knot_id, None, "attested-inherited-provenance", evidence),
        )
        assigned.add(identity)

    for identity in sorted(set(representations) - assigned):
        key, _, _, _ = graph_rows[identity]
        if key is None or key not in vertex_names:
            continue
        knot_id = vertex_names[key]
        evidence = add_evidence(
            db,
            "verified",
            "exact-canonical-stopping-key",
            str(args.lookup_maps),
            file_sha256(args.lookup_maps),
            f"stopping_key={bytes(key).hex()}",
            {"knot_id": knot_id},
        )
        db.execute(
            "INSERT INTO verified_representation_knot_map VALUES (?,?,?,?,?)",
            (identity, knot_id, 0, "verified-canonical-stopping-key", evidence),
        )
        assigned.add(identity)

    for identity in sorted(set(representations) - assigned):
        if identity not in dkt:
            continue
        record = dkt[identity]
        payload = record["payload"]
        entry = representations[identity]
        expected = (tuple(map(int, payload["word"])), int(payload["strands"]))
        actual = (tuple(map(int, entry["word"])), int(entry["strands"]))
        if actual != expected:
            raise ValueError(f"DKT payload mismatch for {identity}")
        replayed_word = tuple(map(int, spherogram.Link(payload["source_pd"]).braid_word()))
        if replayed_word != expected[0]:
            raise ValueError(f"DKT PD-to-braid replay mismatch for {identity}")
        name = str(record["source_id"])
        if re.fullmatch(r"(?:\d+_|1[123][an]_)[0-9]+", name) is None:
            raise ValueError(f"invalid DKT canonical name: {name}")
        knot_id = ensure_knot(name)
        evidence = add_evidence(
            db,
            "verified",
            "replayed-named-pd-to-braid-conversion",
            str(dkt_path.relative_to(args.rf_root)),
            dkt_sha256,
            f"instance_id={identity}",
            {"source_id": name, "source_pd_sha256": payload["source_pd_sha256"]},
        )
        db.execute(
            "INSERT INTO verified_representation_knot_map VALUES (?,?,?,?,?)",
            (identity, knot_id, 0, "verified-source-conversion", evidence),
        )
        assigned.add(identity)

    for identity in sorted(set(representations) - assigned):
        _, node_id, u_upper, _ = graph_rows[identity]
        if node_id is None or u_upper != 0:
            continue
        knot_id = ensure_knot("0_1")
        evidence = add_evidence(
            db,
            "verified",
            "replay-validated-zero-cc-terminal-route",
            str(args.graph_snapshot),
            graph_sha256,
            f"graph_node_id={node_id}",
            {"u_upper": 0, "terminal": "unknot"},
        )
        db.execute(
            "INSERT INTO verified_representation_knot_map VALUES (?,?,?,?,?)",
            (identity, knot_id, 0, "verified-terminal-unknot", evidence),
        )
        assigned.add(identity)

    for identity in sorted(set(representations) - assigned):
        matching_sources = sorted(source_ids(representations[identity]) & LADDER_ATTESTATIONS.keys())
        names = {LADDER_ATTESTATIONS[source] for source in matching_sources}
        if not matching_sources or len(names) != 1:
            continue
        name = next(iter(names))
        knot_id = ensure_knot(name)
        evidence = add_evidence(
            db,
            "attested",
            "rf-generated-rung-identification-report",
            str(ladder_path.relative_to(args.rf_root)),
            ladder_sha256,
            f"source={matching_sources[0]}",
            {"source_ids": matching_sources, "reported_name": name},
        )
        db.execute(
            "INSERT INTO attested_representation_knot_map VALUES (?,?,?,?,?)",
            (identity, knot_id, None, "attested-rf-rung-report", evidence),
        )
        assigned.add(identity)

    candidate_representations = sorted(set(representations) - assigned)
    for index, identity in enumerate(candidate_representations, 1):
        entry = representations[identity]
        word = tuple(map(int, entry["word"]))
        strands = int(entry["strands"])
        alexander = canonical_json(to_pairs(alexander_polynomial(word, strands)))
        jones_pairs = to_pairs(jones_polynomial(word, strands))
        jones = canonical_json(jones_pairs)
        mirror_jones = canonical_json([[-exponent, coefficient] for exponent, coefficient in reversed(jones_pairs)])
        signature_value = signature(word, strands)
        if signature_value is None:
            raise ValueError(f"signature unavailable for {identity}")
        determinant_value = str(determinant(word, strands))
        bundle = {
            "determinant": determinant_value,
            "alexander": alexander,
            "jones": jones,
            "signature": str(signature_value),
        }
        bundle_digest = hashlib.sha256(
            b"UNKNOTDB_IDENTIFICATION_INVARIANT_BUNDLE_V1\0"
            + canonical_json(bundle).encode()
        ).digest()
        db.execute(
            "INSERT INTO representation_invariant_fingerprints VALUES (?,?,?,?,?,?)",
            (
                identity,
                determinant_value,
                alexander,
                jones,
                str(signature_value),
                bundle_digest,
            ),
        )
        candidates: list[tuple[str, int]] = []
        for knot_id, values in known_invariants:
            if values["determinant"] != determinant_value or values["alexander"] != alexander:
                continue
            if values["jones"] == jones and values["signature"] == str(signature_value):
                candidates.append((knot_id, 0))
            elif values["jones"] == mirror_jones and values["signature"] == str(-signature_value):
                candidates.append((knot_id, 1))
        for rank, (knot_id, mirror_bit) in enumerate(sorted(candidates), 1):
            evidence = add_evidence(
                db,
                "candidate",
                "exact-four-invariant-match",
                "self:representation_invariant_fingerprints",
                "self",
                f"representation_id={identity}",
                {
                    "candidate_knot_id": knot_id,
                    "mirror_bit": mirror_bit,
                    "bundle_sha256": bundle_digest.hex(),
                    "candidate_count": len(candidates),
                },
            )
            db.execute(
                "INSERT INTO representation_knot_candidates VALUES (?,?,?,?,?)",
                (identity, knot_id, mirror_bit, rank, evidence),
            )
        if index % 10 == 0:
            print(f"candidate-analysis {index}/{len(candidate_representations)}", flush=True)

    overlap = db.execute(
        """
        SELECT count(*) FROM verified_representation_knot_map v
        JOIN attested_representation_knot_map a USING(representation_id)
        """
    ).fetchone()[0]
    if overlap:
        raise RuntimeError("verified and attested maps overlap")
    promoted_candidates = db.execute(
        """
        SELECT count(*) FROM representation_knot_candidates c
        JOIN effective_representation_knot_map e USING(representation_id)
        """
    ).fetchone()[0]
    if promoted_candidates:
        raise RuntimeError("candidate representation leaked into effective map")
    counts = {
        "representations": db.execute("SELECT count(*) FROM representations").fetchone()[0],
        "verified": db.execute(
            "SELECT count(*) FROM verified_representation_knot_map"
        ).fetchone()[0],
        "attested": db.execute(
            "SELECT count(*) FROM attested_representation_knot_map"
        ).fetchone()[0],
        "effective": db.execute(
            "SELECT count(*) FROM effective_representation_knot_map"
        ).fetchone()[0],
        "candidate_representations": db.execute(
            "SELECT count(DISTINCT representation_id) FROM representation_knot_candidates"
        ).fetchone()[0],
        "candidate_rows": db.execute(
            "SELECT count(*) FROM representation_knot_candidates"
        ).fetchone()[0],
        "unidentified": db.execute(
            """
            SELECT count(*) FROM unidentified_representations u WHERE NOT EXISTS(
                SELECT 1 FROM representation_knot_candidates c
                WHERE c.representation_id=u.representation_id
            )
            """
        ).fetchone()[0],
    }
    if counts != {
        "representations": 3519,
        "verified": 70,
        "attested": 3430,
        "effective": 3500,
        "candidate_representations": 14,
        "candidate_rows": 14,
        "unidentified": 5,
    }:
        raise RuntimeError(f"unexpected identification counts: {counts}")
    candidate_report_rows = list(
        db.execute(
            """
            SELECT representation_id,candidate_knot_id,mirror_bit
            FROM representation_knot_candidates
            ORDER BY representation_id,candidate_rank
            """
        )
    )
    unidentified_report_rows = list(
        db.execute(
            """
            SELECT u.representation_id,u.graph_node_id,u.graph_u_upper
            FROM unidentified_representations u WHERE NOT EXISTS(
                SELECT 1 FROM representation_knot_candidates c
                WHERE c.representation_id=u.representation_id
            ) ORDER BY u.representation_id
            """
        )
    )
    db.execute("PRAGMA optimize")
    integrity = db.execute("PRAGMA integrity_check").fetchone()[0]
    foreign_keys = db.execute("PRAGMA foreign_key_check").fetchall()
    if integrity != "ok" or foreign_keys:
        raise RuntimeError(f"integrity={integrity} foreign_keys={foreign_keys[:3]}")
    db.commit()
    db.close()
    os.replace(temp, args.output)

    output_sha256 = file_sha256(args.output)
    candidate_lines = "\n".join(
        f"- `{identity}` -> `{knot_id}` (mirror_bit={mirror_bit})"
        for identity, knot_id, mirror_bit in candidate_report_rows
    )
    unidentified_lines = "\n".join(
        f"- `{identity}` (sources={sorted(source_ids(representations[identity]))}, "
        f"graph_node={node_id}, U_upper={u_upper})"
        for identity, node_id, u_upper in unidentified_report_rows
    )
    report = f"""# Tier-separated knot identification maps v3

- Sidecar: `{args.output}` ({args.output.stat().st_size:,} bytes)
- SHA-256: `{output_sha256}`
- Representations: {counts['representations']:,}
- Verified mappings: {counts['verified']:,}
- Attested mappings: {counts['attested']:,}
- Effective verified-or-attested mappings: {counts['effective']:,}
- Representations with invariant-only candidates: {counts['candidate_representations']:,}
- Candidate rows: {counts['candidate_rows']:,}
- Fully unidentified representations: {counts['unidentified']:,}
- SQLite integrity and foreign keys: `ok`

Candidates are physically excluded from `effective_representation_knot_map`. Promotion requires a new verified or attested evidence record. Existing v2 provenance mappings remain attested; exact canonical stopping-key matches, replayed DKT named-PD conversions, and replay-validated zero-CC terminal routes are verified.

## Candidate-only mappings

{candidate_lines}

## Fully unidentified

{unidentified_lines}
"""
    args.report.write_text(report)
    print(" ".join(f"{key}={value}" for key, value in counts.items()))
    print(f"bytes={args.output.stat().st_size} sha256={output_sha256}")


if __name__ == "__main__":
    main()
