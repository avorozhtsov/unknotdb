#!/usr/bin/env python3
"""Promote externally simplified zero-crossing source knots in a new sidecar."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import sqlite3
import time
from pathlib import Path

from build_graph_identification_sidecar import add_evidence, canonical_json, file_sha256
from check_identification_equivalence import load_source_words, regina_link


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--corpus-json", action="append", required=True, type=Path)
    parser.add_argument("--limit", type=int, default=100)
    args = parser.parse_args()
    if args.output.exists() or args.report.exists():
        raise FileExistsError("output artifact already exists")
    started = time.monotonic()
    input_sha256 = file_sha256(args.input)
    temp = args.output.with_name(args.output.name + f".tmp-{os.getpid()}")
    shutil.copyfile(args.input, temp)
    db = sqlite3.connect(temp)
    db.execute("PRAGMA foreign_keys=ON")
    schema = db.execute("SELECT value FROM meta WHERE key='schema'").fetchone()
    if schema != ("unknotdb-graph-identification-v5",):
        raise ValueError(f"unsupported input schema: {schema}")

    import regina
    import snappy

    words = load_source_words(args.corpus_json)
    rows = list(
        db.execute(
            """
            SELECT u.representation_id,u.stopping_key,u.graph_u_upper
            FROM unidentified_representations u
            WHERE NOT EXISTS(
                SELECT 1 FROM representation_knot_candidates c
                WHERE c.representation_id=u.representation_id
            )
            ORDER BY u.representation_id LIMIT ?
            """,
            (args.limit,),
        )
    )
    checked = 0
    promoted_sources = 0
    promoted_graph_vertices = 0
    discoveries: list[dict[str, object]] = []
    unknot_id = "knot:0_1"
    if (
        db.execute("SELECT 1 FROM knot_ids WHERE knot_id=?", (unknot_id,)).fetchone()
        is None
    ):
        db.execute(
            "INSERT INTO knot_ids VALUES (?,?,?)",
            (unknot_id, "0_1", "canonical-catalogue-name-v1"),
        )
    for representation_id, stopping_key, graph_u in rows:
        _, word = words[str(representation_id)]
        link = regina_link(word, snappy, regina)
        initial_crossings = int(link.size())
        initial_signature = link.sig()
        changed = bool(link.simplify())
        checked += 1
        discovery = {
            "representation_id": representation_id,
            "initial_crossings": initial_crossings,
            "initial_signature": initial_signature,
            "simplification_changed": changed,
            "final_crossings": int(link.size()),
            "final_components": int(link.countComponents()),
            "final_signature": link.sig(),
            "graph_u_upper": graph_u,
        }
        discoveries.append(discovery)
        if int(link.size()) != 0 or int(link.countComponents()) != 1:
            continue
        evidence = add_evidence(
            db,
            "attested",
            "external-regina-zero-crossing-simplification",
            str(args.input),
            input_sha256,
            f"representation_id={representation_id}",
            {
                **discovery,
                "regina_version": regina.versionString(),
                "trust": "external-attestation-not-proof-graph-replay",
            },
        )
        db.execute(
            "INSERT OR IGNORE INTO attested_representation_knot_map VALUES (?,?,?,?,?)",
            (
                representation_id,
                unknot_id,
                None,
                "attested-regina-zero-crossing-unknot",
                evidence,
            ),
        )
        promoted_sources += db.execute("SELECT changes()").fetchone()[0]
        if stopping_key is None:
            continue
        component = db.execute(
            "SELECT cc0_component_key FROM graph_vertices WHERE rep_key=?",
            (stopping_key,),
        ).fetchone()
        if component is None:
            continue
        vertices = list(
            db.execute(
                "SELECT rep_key FROM graph_vertices WHERE cc0_component_key=?",
                (component[0],),
            )
        )
        for (rep_key,) in vertices:
            existing = db.execute(
                "SELECT knot_id FROM graph_vertex_knot_map WHERE rep_key=?", (rep_key,)
            ).fetchone()
            if existing is not None and existing != (unknot_id,):
                raise RuntimeError(
                    "Regina unknot discovery conflicts with graph mapping"
                )
            db.execute(
                "INSERT OR IGNORE INTO graph_vertex_knot_map VALUES (?,?,?,?,?,?)",
                (
                    rep_key,
                    unknot_id,
                    None,
                    "attested",
                    "attested-regina-unknot-and-cc0",
                    evidence,
                ),
            )
            promoted_graph_vertices += db.execute("SELECT changes()").fetchone()[0]

    counts = {
        "checked": checked,
        "promoted_sources": promoted_sources,
        "promoted_graph_vertices": promoted_graph_vertices,
        "effective_source_mappings": db.execute(
            "SELECT count(*) FROM effective_representation_knot_map"
        ).fetchone()[0],
        "mapped_graph_vertices": db.execute(
            "SELECT count(*) FROM graph_vertex_knot_map"
        ).fetchone()[0],
        "postings": db.execute(
            "SELECT count(*) FROM knot_representation_postings"
        ).fetchone()[0],
    }
    db.executemany(
        "INSERT OR REPLACE INTO meta VALUES (?,?)",
        [
            ("regina_unknot_parent_sha256", input_sha256),
            ("regina_unknot_discoveries", canonical_json(discoveries)),
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
        "# Regina bounded unknot discovery\n\n"
        f"- Input: `{args.input}` (SHA-256 `{input_sha256}`)\n"
        f"- Output: `{args.output}` ({args.output.stat().st_size:,} bytes; SHA-256 `{output_sha256}`)\n"
        f"- Elapsed: {elapsed:.1f} seconds\n"
        + "\n".join(
            f"- {key.replace('_', ' ').title()}: {value:,}"
            for key, value in counts.items()
        )
        + "\n- SQLite integrity and foreign keys: `ok`\n\n"
        "The promoted unknot is an external Regina attestation. Its graph vertex still has a stale positive U upper bound; repairing that proof-graph route is separate work.\n\n"
        "```json\n" + json.dumps(discoveries, indent=2, sort_keys=True) + "\n```\n"
    )
    print(
        canonical_json({**counts, "elapsed_seconds": elapsed, "sha256": output_sha256})
    )


if __name__ == "__main__":
    main()
