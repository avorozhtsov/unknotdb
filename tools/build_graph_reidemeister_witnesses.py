#!/usr/bin/env python3
"""Search exact CC plus planar Reidemeister witnesses for stored graph vertices."""

from __future__ import annotations

import argparse
import csv
import hashlib
import itertools
import json
import sqlite3
import time
from pathlib import Path
from typing import Any

from build_graph_identification_sidecar import decode_representation
from reidemeister_trace import extract_seeded_level_trace, extract_trace

FORMAT = "unknotdb-catalogue-reidemeister-witness-cohort-v0"


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--source-key", action="append", required=True)
    parser.add_argument("--knot-id", required=True)
    parser.add_argument("--cc-count", type=int, required=True)
    parser.add_argument("--max-type-iii", type=int, default=250)
    parser.add_argument("--seeded-fallback-seeds", type=int, default=0)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--summary", type=Path, required=True)
    args = parser.parse_args()
    if args.cc_count < 0:
        raise ValueError("--cc-count must be non-negative")
    if args.output.exists() or args.summary.exists():
        raise FileExistsError("output artifact already exists")

    graph = sqlite3.connect(f"file:{args.graph}?mode=ro", uri=True)
    started = time.monotonic()
    results: list[dict[str, Any]] = []
    try:
        for key_hex in args.source_key:
            key = bytes.fromhex(key_hex)
            row = graph.execute(
                """
                SELECT n.node_id,n.u_upper_bound,r.encoding
                FROM node_keys k JOIN nodes n USING(node_id)
                JOIN representations r USING(node_id)
                WHERE k.rep_key=?
                """,
                (key,),
            ).fetchone()
            if row is None:
                raise ValueError(f"source key is absent from graph: {key_hex}")
            node_id, graph_u, encoding = row
            strands, cyclic_band, decoded = decode_representation(bytes(encoding))
            if cyclic_band:
                raise ValueError(
                    f"planar fallback does not support cyclic bands: {key_hex}"
                )
            word = list(decoded)
            attempts = 0
            scheduler_attempts = 0
            witness = None
            last_error = None
            for positions in itertools.combinations(range(len(word)), args.cc_count):
                attempts += 1
                changed = word.copy()
                for position in positions:
                    changed[position] = -changed[position]
                try:
                    trace = extract_trace(changed, args.max_type_iii)
                except ValueError as error:
                    last_error = str(error)
                    continue
                witness = {
                    "crossing_change_positions": list(positions),
                    "post_cc_word": changed,
                    "zero_cc_trace": trace,
                }
                break
            if witness is None and args.seeded_fallback_seeds:
                for positions in itertools.combinations(
                    range(len(word)), args.cc_count
                ):
                    changed = word.copy()
                    for position in positions:
                        changed[position] = -changed[position]
                    trace = None
                    for seed in range(args.seeded_fallback_seeds):
                        scheduler_attempts += 1
                        try:
                            trace = extract_seeded_level_trace(
                                changed, seed, args.max_type_iii
                            )
                            break
                        except ValueError as error:
                            last_error = str(error)
                    if trace is not None:
                        witness = {
                            "crossing_change_positions": list(positions),
                            "post_cc_word": changed,
                            "zero_cc_trace": trace,
                        }
                        break
            results.append(
                {
                    "knot_id": args.knot_id,
                    "catalogue_exact_u": args.cc_count,
                    "standard_braid": {
                        "strands": strands,
                        "word": word,
                        "normalized_word": word,
                        "normalization_mirrored": False,
                        "rep_key": key.hex(),
                    },
                    "graph": {
                        "node_id": int(node_id),
                        "u_upper": int(graph_u),
                        "exact_key_hit": True,
                    },
                    "search": {
                        "attempts": attempts,
                        "scheduler_attempts": scheduler_attempts,
                        "status": "witness" if witness else "miss",
                        "last_error": None if witness else last_error,
                    },
                    "witness": witness,
                }
            )
    finally:
        graph.close()

    artifact = {
        "format": FORMAT,
        "inputs": {
            "graph": str(args.graph),
            "graph_sha256": file_sha256(args.graph),
            "source_keys": [key.lower() for key in args.source_key],
            "knot_id": args.knot_id,
            "cc_count": args.cc_count,
            "max_type_iii": args.max_type_iii,
            "seeded_fallback_seeds": args.seeded_fallback_seeds,
        },
        "summary": {
            "selected": len(results),
            "witnesses": sum(item["witness"] is not None for item in results),
            "misses": sum(item["witness"] is None for item in results),
            "exact_graph_key_hits": len(results),
            "elapsed_seconds": time.monotonic() - started,
        },
        "results": results,
    }
    args.output.write_text(json.dumps(artifact, indent=2) + "\n")
    with args.summary.open("w", newline="") as stream:
        writer = csv.writer(stream, delimiter="\t", lineterminator="\n")
        writer.writerow(
            (
                "source_key",
                "node_id",
                "old_u",
                "status",
                "attempts",
                "cc_positions",
                "zero_ri",
                "zero_rii",
                "zero_riii",
            )
        )
        for item in results:
            witness = item["witness"]
            moves = witness["zero_cc_trace"]["moves"] if witness else []
            writer.writerow(
                (
                    item["standard_braid"]["rep_key"],
                    item["graph"]["node_id"],
                    item["graph"]["u_upper"],
                    item["search"]["status"],
                    item["search"]["attempts"],
                    ",".join(map(str, witness["crossing_change_positions"]))
                    if witness
                    else "",
                    sum(move["move"] == "RI" for move in moves),
                    sum(move["move"] == "RII" for move in moves),
                    sum(move["move"] == "RIII" for move in moves),
                )
            )
    print(json.dumps(artifact["summary"], sort_keys=True))


if __name__ == "__main__":
    main()
