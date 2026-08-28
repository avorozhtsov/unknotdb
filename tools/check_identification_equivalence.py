#!/usr/bin/env python3
"""Bounded external equivalence checks for identification candidates.

This tool deliberately writes a new sidecar.  Regina/SnapPy results are
recorded as external attestations, never as proof-graph edges.
"""

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

from build_graph_identification_sidecar import (
    add_evidence,
    canonical_json,
    decode_representation,
    file_sha256,
)

SCHEMA = "unknotdb-graph-identification-v5"


def load_source_words(paths: list[Path]) -> dict[str, tuple[int, tuple[int, ...]]]:
    result: dict[str, tuple[int, tuple[int, ...]]] = {}
    for path in paths:
        for entry in json.loads(path.read_text())["entries"]:
            candidate = (int(entry["strands"]), tuple(map(int, entry["word"])))
            previous = result.setdefault(str(entry["representation_id"]), candidate)
            if previous != candidate:
                raise ValueError(
                    f"conflicting source representation {entry['representation_id']}"
                )
    return result


def regina_link(word: tuple[int, ...], snappy: Any, regina: Any) -> Any:
    link = snappy.Link(braid_closure=list(word))
    pd = [[int(label) + 1 for label in crossing] for crossing in link.PD_code()]
    return regina.Link.fromPD(str(pd))


def named_regina_link(name: str, snappy: Any, regina: Any) -> Any:
    link = snappy.Link(name)
    pd = [[int(label) + 1 for label in crossing] for crossing in link.PD_code()]
    return regina.Link.fromPD(str(pd))


def bounded_equivalence(
    left: Any,
    right: Any,
    *,
    exhaustive_height: int,
    exhaustive_crossings: int,
) -> tuple[bool, str, dict[str, Any]]:
    details: dict[str, Any] = {
        "left_initial_crossings": int(left.size()),
        "right_initial_crossings": int(right.size()),
        "left_initial_sig": left.sig(),
        "right_initial_sig": right.sig(),
    }
    if details["left_initial_sig"] == details["right_initial_sig"]:
        return True, "regina-diagram-isomorphism", details
    left.simplify()
    right.simplify()
    details.update(
        left_simplified_crossings=int(left.size()),
        right_simplified_crossings=int(right.size()),
        left_simplified_sig=left.sig(),
        right_simplified_sig=right.sig(),
    )
    if details["left_simplified_sig"] == details["right_simplified_sig"]:
        return True, "regina-reidemeister-simplified-diagram", details
    if max(int(left.size()), int(right.size())) <= exhaustive_crossings:
        left.simplifyExhaustive(exhaustive_height, 1)
        right.simplifyExhaustive(exhaustive_height, 1)
        details.update(
            left_exhaustive_crossings=int(left.size()),
            right_exhaustive_crossings=int(right.size()),
            left_exhaustive_sig=left.sig(),
            right_exhaustive_sig=right.sig(),
        )
        if details["left_exhaustive_sig"] == details["right_exhaustive_sig"]:
            return True, "regina-bounded-reidemeister-search", details
    left_complement = left.complement()
    right_complement = right.complement()
    left_complement.intelligentSimplify()
    right_complement.intelligentSimplify()
    details.update(
        left_complement_isosig=left_complement.isoSig(),
        right_complement_isosig=right_complement.isoSig(),
    )
    if details["left_complement_isosig"] == details["right_complement_isosig"]:
        return True, "regina-complement-combinatorial-isomorphism", details
    return False, "bounded-no-match", details


def check_id(payload: dict[str, Any]) -> bytes:
    return hashlib.sha256(
        b"UNKNOTDB_EXTERNAL_EQUIVALENCE_CHECK_V0\0" + canonical_json(payload).encode()
    ).digest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--corpus-json", action="append", required=True, type=Path)
    parser.add_argument("--max-graph-candidates", type=int, default=1000)
    parser.add_argument("--max-source-candidates", type=int, default=100)
    parser.add_argument("--exhaustive-height", type=int, default=1)
    parser.add_argument("--exhaustive-crossings", type=int, default=14)
    parser.add_argument("--deadline-seconds", type=int, default=7200)
    args = parser.parse_args()
    if args.output.exists() or args.report.exists():
        raise FileExistsError("output artifact already exists")
    started = time.monotonic()
    deadline = started + args.deadline_seconds
    input_sha256 = file_sha256(args.input)
    temp = args.output.with_name(args.output.name + f".tmp-{os.getpid()}")
    shutil.copyfile(args.input, temp)
    db = sqlite3.connect(temp)
    db.execute("PRAGMA foreign_keys=ON")
    schema = db.execute("SELECT value FROM meta WHERE key='schema'").fetchone()
    if schema not in {
        ("unknotdb-graph-identification-v4",),
        ("unknotdb-graph-identification-v5",),
    }:
        raise ValueError(f"unsupported input schema: {schema}")

    import regina
    import snappy

    engine = {
        "regina_version": regina.versionString(),
        "snappy_version": snappy.__version__,
        "python": sys.version,
        "exhaustive_height": args.exhaustive_height,
        "exhaustive_crossings": args.exhaustive_crossings,
    }
    db.executescript(
        """
        CREATE TABLE IF NOT EXISTS graph_equivalence_checks(
            check_id BLOB PRIMARY KEY CHECK(length(check_id)=32),
            candidate_id BLOB NOT NULL,
            matched INTEGER NOT NULL CHECK(matched IN (0,1)),
            method TEXT NOT NULL,
            engine_json TEXT NOT NULL,
            details_json TEXT NOT NULL,
            elapsed_ms INTEGER NOT NULL,
            FOREIGN KEY(candidate_id) REFERENCES graph_equivalence_candidates(candidate_id)
        ) WITHOUT ROWID;
        CREATE TABLE IF NOT EXISTS source_equivalence_checks(
            check_id BLOB PRIMARY KEY CHECK(length(check_id)=32),
            representation_id TEXT NOT NULL,
            candidate_knot_id TEXT NOT NULL,
            matched INTEGER NOT NULL CHECK(matched IN (0,1)),
            method TEXT NOT NULL,
            engine_json TEXT NOT NULL,
            details_json TEXT NOT NULL,
            elapsed_ms INTEGER NOT NULL,
            FOREIGN KEY(representation_id) REFERENCES representations(representation_id),
            FOREIGN KEY(candidate_knot_id) REFERENCES knot_ids(knot_id)
        ) WITHOUT ROWID;
        """
    )

    graph_rows = list(
        db.execute(
            """
            SELECT c.candidate_id,c.left_rep_key,c.right_rep_key,c.candidate_knot_id,
                   lv.encoding,rv.encoding,lv.cc0_component_key
            FROM graph_equivalence_candidates c
            JOIN graph_vertices lv ON lv.rep_key=c.left_rep_key
            JOIN graph_vertices rv ON rv.rep_key=c.right_rep_key
            WHERE c.status='pending' ORDER BY c.candidate_rank,c.candidate_id
            LIMIT ?
            """,
            (args.max_graph_candidates,),
        )
    )
    graph_matches: dict[bytes, list[tuple[bytes, str, str, dict[str, Any]]]] = (
        defaultdict(list)
    )
    graph_checked = 0
    graph_matched = 0
    graph_errors = 0
    for (
        candidate,
        left_key,
        right_key,
        knot_id,
        left_encoding,
        right_encoding,
        component,
    ) in graph_rows:
        if time.monotonic() >= deadline:
            break
        check_started = time.monotonic()
        try:
            _, left_cyclic, left_word = decode_representation(bytes(left_encoding))
            _, right_cyclic, right_word = decode_representation(bytes(right_encoding))
            if left_cyclic or right_cyclic:
                raise ValueError("external checker supports ordinary Artin braids only")
            matched, method, details = bounded_equivalence(
                regina_link(left_word, snappy, regina),
                regina_link(right_word, snappy, regina),
                exhaustive_height=args.exhaustive_height,
                exhaustive_crossings=args.exhaustive_crossings,
            )
        except Exception as error:  # noqa: BLE001 - durable per-row failure
            matched = False
            method = "external-error"
            details = {"error_type": type(error).__name__, "error": str(error)}
            graph_errors += 1
        elapsed_ms = round((time.monotonic() - check_started) * 1000)
        payload = {
            "candidate_id": bytes(candidate).hex(),
            "matched": matched,
            "method": method,
            "engine": engine,
            "details": details,
        }
        identity = check_id(payload)
        db.execute(
            "INSERT INTO graph_equivalence_checks VALUES (?,?,?,?,?,?,?)",
            (
                identity,
                candidate,
                int(matched),
                method,
                canonical_json(engine),
                canonical_json(details),
                elapsed_ms,
            ),
        )
        graph_checked += 1
        if matched:
            graph_matched += 1
            graph_matches[bytes(component)].append(
                (bytes(candidate), str(knot_id), method, details)
            )

    promoted_graph_components = 0
    promoted_graph_vertices = 0
    ambiguous_graph_components = 0
    graph_snapshot_sha256 = db.execute(
        "SELECT value FROM meta WHERE key='graph_snapshot_sha256'"
    ).fetchone()[0]
    for component, matches in graph_matches.items():
        knot_ids = {knot_id for _, knot_id, _, _ in matches}
        if len(knot_ids) != 1:
            ambiguous_graph_components += 1
            continue
        knot_id = next(iter(knot_ids))
        evidence = add_evidence(
            db,
            "attested",
            "external-regina-equivalence-check",
            str(args.input),
            input_sha256,
            f"cc0_component={component.hex()}",
            {
                "knot_id": knot_id,
                "checks": [
                    {"candidate_id": candidate.hex(), "method": method}
                    for candidate, _, method, _ in matches
                ],
                "engine": engine,
                "trust": "external-attestation-not-proof-graph-replay",
            },
        )
        vertices = list(
            db.execute(
                "SELECT rep_key FROM graph_vertices WHERE cc0_component_key=?",
                (component,),
            )
        )
        for (rep_key,) in vertices:
            db.execute(
                "INSERT OR IGNORE INTO graph_vertex_knot_map VALUES (?,?,?,?,?,?)",
                (
                    rep_key,
                    knot_id,
                    None,
                    "attested",
                    "attested-external-equivalence-and-cc0",
                    evidence,
                ),
            )
        for candidate, _, _, _ in matches:
            db.execute(
                "UPDATE graph_equivalence_candidates SET status='attested',evidence_id=? WHERE candidate_id=?",
                (evidence, candidate),
            )
        promoted_graph_components += 1
        promoted_graph_vertices += len(vertices)

    source_words = load_source_words(args.corpus_json)
    source_rows = list(
        db.execute(
            """
            SELECT c.representation_id,c.candidate_knot_id,k.canonical_name
            FROM representation_knot_candidates c
            JOIN knot_ids k ON k.knot_id=c.candidate_knot_id
            WHERE NOT EXISTS(
                SELECT 1 FROM effective_representation_knot_map m
                WHERE m.representation_id=c.representation_id
            )
            ORDER BY c.candidate_rank,c.representation_id LIMIT ?
            """,
            (args.max_source_candidates,),
        )
    )
    source_checked = 0
    source_matched = 0
    source_errors = 0
    source_promoted = 0
    for representation_id, knot_id, canonical_name in source_rows:
        if time.monotonic() >= deadline:
            break
        check_started = time.monotonic()
        try:
            _, word = source_words[str(representation_id)]
            matched, method, details = bounded_equivalence(
                regina_link(word, snappy, regina),
                named_regina_link(str(canonical_name), snappy, regina),
                exhaustive_height=args.exhaustive_height,
                exhaustive_crossings=args.exhaustive_crossings,
            )
        except Exception as error:  # noqa: BLE001 - durable per-row failure
            matched = False
            method = "external-error"
            details = {"error_type": type(error).__name__, "error": str(error)}
            source_errors += 1
        elapsed_ms = round((time.monotonic() - check_started) * 1000)
        payload = {
            "representation_id": representation_id,
            "candidate_knot_id": knot_id,
            "matched": matched,
            "method": method,
            "engine": engine,
            "details": details,
        }
        identity = check_id(payload)
        db.execute(
            "INSERT INTO source_equivalence_checks VALUES (?,?,?,?,?,?,?,?)",
            (
                identity,
                representation_id,
                knot_id,
                int(matched),
                method,
                canonical_json(engine),
                canonical_json(details),
                elapsed_ms,
            ),
        )
        source_checked += 1
        if not matched:
            continue
        source_matched += 1
        evidence = add_evidence(
            db,
            "attested",
            "external-regina-named-knot-equivalence",
            str(args.input),
            input_sha256,
            f"representation_id={representation_id}",
            {
                "knot_id": knot_id,
                "method": method,
                "engine": engine,
                "trust": "external-attestation-not-proof-graph-replay",
            },
        )
        db.execute(
            "INSERT OR IGNORE INTO attested_representation_knot_map VALUES (?,?,?,?,?)",
            (
                representation_id,
                knot_id,
                None,
                "attested-external-regina-equivalence",
                evidence,
            ),
        )
        source_promoted += db.execute("SELECT changes()").fetchone()[0]

    db.execute("UPDATE meta SET value=? WHERE key='schema'", (SCHEMA,))
    counts = {
        "graph_candidates_checked": graph_checked,
        "graph_candidates_matched": graph_matched,
        "graph_check_errors": graph_errors,
        "promoted_graph_components": promoted_graph_components,
        "promoted_graph_vertices": promoted_graph_vertices,
        "ambiguous_graph_components": ambiguous_graph_components,
        "source_candidates_checked": source_checked,
        "source_candidates_matched": source_matched,
        "source_check_errors": source_errors,
        "source_representations_promoted": source_promoted,
        "effective_source_mappings": db.execute(
            "SELECT count(*) FROM effective_representation_knot_map"
        ).fetchone()[0],
        "mapped_graph_vertices": db.execute(
            "SELECT count(*) FROM graph_vertex_knot_map"
        ).fetchone()[0],
        "pending_graph_candidates": db.execute(
            "SELECT count(*) FROM graph_equivalence_candidates WHERE status='pending'"
        ).fetchone()[0],
        "postings": db.execute(
            "SELECT count(*) FROM knot_representation_postings"
        ).fetchone()[0],
    }
    db.executemany(
        "INSERT OR REPLACE INTO meta VALUES (?,?)",
        [
            ("external_checker_parent_sha256", input_sha256),
            ("external_checker_engine", canonical_json(engine)),
            ("external_checker_graph_snapshot_sha256", graph_snapshot_sha256),
            *((f"count_{key}", str(value)) for key, value in counts.items()),
        ],
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
    elapsed = time.monotonic() - started
    args.report.write_text(
        "# Bounded external knot-equivalence checks v5\n\n"
        f"- Input: `{args.input}` (SHA-256 `{input_sha256}`)\n"
        f"- Output: `{args.output}` ({args.output.stat().st_size:,} bytes; SHA-256 `{output_sha256}`)\n"
        f"- Engine: `{canonical_json(engine)}`\n"
        f"- Elapsed: {elapsed:.1f} seconds\n"
        + "\n".join(
            f"- {key.replace('_', ' ').title()}: {value:,}"
            for key, value in counts.items()
        )
        + "\n- SQLite integrity and foreign keys: `ok`\n\n"
        "Promotions are external attestations. They do not create proof-graph edges and are not treated as independently replayed UnknotDB certificates.\n"
    )
    print(
        canonical_json({**counts, "elapsed_seconds": elapsed, "sha256": output_sha256})
    )


if __name__ == "__main__":
    main()
