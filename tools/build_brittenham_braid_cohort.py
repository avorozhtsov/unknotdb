#!/usr/bin/env python3
"""Build a deterministic cohort of Brittenham claims that braidify to one CC.

This is an audit/selection tool.  It does not mutate either the catalogue
sidecar or the proof graph.  A row is emitted only when Spherogram braidifies
the recorded source DT code and its recorded one-sign successor to braid words
with identical absolute generators and exactly one opposite sign.
"""

from __future__ import annotations

import argparse
import ast
import csv
import hashlib
import json
import sqlite3
from pathlib import Path

import spherogram


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def braid_word(dt: list[int]) -> list[int]:
    return [int(value) for value in spherogram.Link("DT:" + str(dt)).braid_word()]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--sidecar", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--limit", type=int, default=256)
    parser.add_argument("--scan-limit", type=int, default=20_000)
    args = parser.parse_args()
    for output in (args.output, args.manifest):
        if output.exists():
            raise FileExistsError(output)

    db = sqlite3.connect(args.sidecar)
    rows = db.execute(
        """
        WITH best_u AS (
          SELECT knot_pk,MIN(graph_u_upper) AS u
          FROM graph_knot_links WHERE graph_u_upper IS NOT NULL GROUP BY knot_pk
        )
        SELECT hex(a.claim_id),ks.canonical_id,kt.canonical_id,
               rs.representation_text,a.crossing_locator,su.u,tu.u,a.source_pointer
        FROM adjacency_claims a
        JOIN knots ks ON ks.knot_pk=a.source_knot_pk
        JOIN knots kt ON kt.knot_pk=a.target_knot_pk
        JOIN representations rs ON rs.representation_pk=a.source_representation_pk
        JOIN best_u su ON su.knot_pk=a.source_knot_pk
        JOIN best_u tu ON tu.knot_pk=a.target_knot_pk
        WHERE a.status='diagram_attested' AND 1+tu.u < su.u
        ORDER BY (su.u-(1+tu.u)) DESC,su.u DESC,
                 ks.canonical_id,kt.canonical_id,a.claim_id
        LIMIT ?
        """,
        (args.scan_limit,),
    ).fetchall()
    db.close()

    emitted: list[dict[str, object]] = []
    counts = {
        "ranked_strict_candidates_scanned": len(rows),
        "global_mirror_skipped": 0,
        "braidification_error": 0,
        "not_single_braid_cc": 0,
        "direct_braid_claims": 0,
    }
    for (
        claim_id,
        source,
        target,
        source_text,
        locator_text,
        source_u,
        target_u,
        pointer,
    ) in rows:
        locator = json.loads(locator_text)
        if locator["global_mirror_applied"]:
            counts["global_mirror_skipped"] += 1
            continue
        source_dt = list(ast.literal_eval(source_text))
        successor_dt = [int(value) for value in locator["raw_successor_dt"]]
        try:
            source_word = braid_word(source_dt)
            target_word = braid_word(successor_dt)
        except (AssertionError, IndexError, RuntimeError, TypeError, ValueError):
            counts["braidification_error"] += 1
            continue
        differences = [
            index
            for index, (left, right) in enumerate(zip(source_word, target_word))
            if left != right
        ]
        if (
            len(source_word) != len(target_word)
            or [abs(value) for value in source_word]
            != [abs(value) for value in target_word]
            or len(differences) != 1
            or source_word[differences[0]] != -target_word[differences[0]]
        ):
            counts["not_single_braid_cc"] += 1
            continue
        counts["direct_braid_claims"] += 1
        strands = max((abs(value) for value in source_word), default=0) + 1
        emitted.append(
            {
                "claim_id": claim_id.lower(),
                "source_knot": source,
                "target_knot": target,
                "source_u_by_name": source_u,
                "target_u_by_name": target_u,
                "candidate_u_by_name": 1 + target_u,
                "gap_by_name": source_u - (1 + target_u),
                "strands": strands,
                "source_word": ",".join(map(str, source_word)),
                "cc_position": differences[0],
                "target_word": ",".join(map(str, target_word)),
                "source_pointer": pointer,
            }
        )
        if len(emitted) == args.limit:
            break

    fieldnames = (
        list(emitted[0])
        if emitted
        else [
            "claim_id",
            "source_knot",
            "target_knot",
            "source_u_by_name",
            "target_u_by_name",
            "candidate_u_by_name",
            "gap_by_name",
            "strands",
            "source_word",
            "cc_position",
            "target_word",
            "source_pointer",
        ]
    )
    with args.output.open("w", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=fieldnames, delimiter="\t")
        writer.writeheader()
        writer.writerows(emitted)
    manifest = {
        "format": "unknotdb-brittenham-direct-braid-cohort-v0",
        "sidecar": str(args.sidecar),
        "sidecar_sha256": file_sha256(args.sidecar),
        "selection": "strict name-level Bellman gap, descending gap/source_u/name/claim_id",
        "limit": args.limit,
        "scan_limit": args.scan_limit,
        "counts": counts,
        "emitted": len(emitted),
        "cohort_sha256": file_sha256(args.output),
        "spherogram_version": getattr(spherogram, "__version__", "unknown"),
    }
    args.manifest.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
