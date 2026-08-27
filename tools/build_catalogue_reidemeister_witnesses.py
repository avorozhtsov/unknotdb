#!/usr/bin/env python3
"""Build a fixed catalogue cohort of exact CC plus Reidemeister witnesses."""

from __future__ import annotations

import argparse
import csv
import hashlib
import itertools
import json
import re
import sqlite3
import struct
import time
from pathlib import Path
from typing import Any

from reidemeister_trace import extract_seeded_level_trace, extract_trace

FORMAT = "unknotdb-catalogue-reidemeister-witness-cohort-v0"
KNOT_ID = re.compile(r"knot:(\d+)_(\d+)$")


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def minimal_rotation(word: list[int]) -> tuple[int, ...]:
    if not word:
        return ()
    return min(tuple(word[offset:] + word[:offset]) for offset in range(len(word)))


def normalize_word(word: list[int]) -> tuple[list[int], bool]:
    original = minimal_rotation(word)
    mirrored = minimal_rotation([-letter for letter in word])
    writhe = sum(word)
    if writhe > 0:
        return list(original), False
    if writhe < 0:
        return list(mirrored), True
    if mirrored < original:
        return list(mirrored), True
    return list(original), False


def rep_key(strands: int, word: list[int]) -> bytes:
    encoded = (
        b"UKB0"
        + bytes((0, 0))
        + struct.pack("<H", strands)
        + struct.pack("<I", len(word))
        + b"".join(struct.pack("<h", letter) for letter in word)
    )
    return hashlib.sha256(encoded).digest()


def catalogue_items(
    catalogue: sqlite3.Connection, min_crossings: int, max_crossings: int
) -> list[tuple[int, int, str, int]]:
    result = []
    for knot_id, lower, upper, claim_kind in catalogue.execute(
        "SELECT knot_id,u_lower,u_upper,claim_kind FROM catalogue_u_claims"
    ):
        match = KNOT_ID.fullmatch(knot_id)
        if not match or claim_kind != "exact" or lower != upper:
            continue
        crossings, index = map(int, match.groups())
        if min_crossings <= crossings <= max_crossings:
            result.append((crossings, index, knot_id, int(upper)))
    result.sort()
    return result


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--catalogue", type=Path, required=True)
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--min-crossings", type=int, required=True)
    parser.add_argument("--max-crossings", type=int, required=True)
    parser.add_argument("--max-type-iii", type=int, default=250)
    parser.add_argument("--seeded-fallback-seeds", type=int, default=0)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--summary", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists() or args.summary.exists():
        raise FileExistsError("output artifact already exists")

    try:
        from spherogram import Link
    except ImportError as error:
        raise SystemExit("Spherogram is required") from error

    catalogue = sqlite3.connect(f"file:{args.catalogue}?mode=ro", uri=True)
    graph = sqlite3.connect(f"file:{args.graph}?mode=ro", uri=True)
    items = catalogue_items(catalogue, args.min_crossings, args.max_crossings)
    started = time.monotonic()
    results: list[dict[str, Any]] = []

    for crossings, index, knot_id, exact_u in items:
        name = f"{crossings}_{index}"
        source_word = list(map(int, Link(name).braid_word()))
        strands = max(map(abs, source_word)) + 1
        word, mirrored = normalize_word(source_word)
        key = rep_key(strands, word)
        graph_row = graph.execute(
            "SELECT n.node_id,n.u_upper_bound FROM node_keys k "
            "JOIN nodes n ON n.node_id=k.node_id WHERE k.rep_key=?",
            (key,),
        ).fetchone()
        attempts = 0
        scheduler_attempts = 0
        witness = None
        last_error = None
        for positions in itertools.combinations(range(len(word)), exact_u):
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
            for positions in itertools.combinations(range(len(word)), exact_u):
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
                    except ValueError as seeded_error:
                        last_error = str(seeded_error)
                if trace is not None:
                    witness = {
                        "crossing_change_positions": list(positions),
                        "post_cc_word": changed,
                        "zero_cc_trace": trace,
                    }
                    break
        result = {
            "knot_id": knot_id,
            "catalogue_exact_u": exact_u,
            "standard_braid": {
                "strands": strands,
                "word": source_word,
                "normalized_word": word,
                "normalization_mirrored": mirrored,
                "rep_key": key.hex(),
            },
            "graph": {
                "node_id": graph_row[0] if graph_row else None,
                "u_upper": graph_row[1] if graph_row else None,
                "exact_key_hit": graph_row is not None,
            },
            "search": {
                "attempts": attempts,
                "scheduler_attempts": scheduler_attempts,
                "status": "witness" if witness else "miss",
                "last_error": None if witness else last_error,
            },
            "witness": witness,
        }
        results.append(result)
        print(
            f"{knot_id}\t{result['search']['status']}\t"
            f"u={exact_u}\tattempts={attempts}\tgraph_hit={graph_row is not None}",
            flush=True,
        )

    artifact = {
        "format": FORMAT,
        "inputs": {
            "catalogue": str(args.catalogue),
            "catalogue_sha256": file_sha256(args.catalogue),
            "graph": str(args.graph),
            "graph_sha256": file_sha256(args.graph),
            "min_crossings": args.min_crossings,
            "max_crossings": args.max_crossings,
            "max_type_iii": args.max_type_iii,
            "seeded_fallback_seeds": args.seeded_fallback_seeds,
        },
        "summary": {
            "selected": len(results),
            "witnesses": sum(item["witness"] is not None for item in results),
            "misses": sum(item["witness"] is None for item in results),
            "exact_graph_key_hits": sum(
                item["graph"]["exact_key_hit"] for item in results
            ),
            "elapsed_seconds": time.monotonic() - started,
        },
        "results": results,
    }
    args.output.write_text(json.dumps(artifact, indent=2) + "\n")
    with args.summary.open("w", newline="") as stream:
        writer = csv.writer(stream, delimiter="\t", lineterminator="\n")
        writer.writerow(
            [
                "knot_id",
                "catalogue_exact_u",
                "status",
                "attempts",
                "strands",
                "word_length",
                "rep_key",
                "graph_node_id",
                "graph_u_upper",
                "cc_positions",
                "zero_ri",
                "zero_rii",
                "zero_riii",
            ]
        )
        for item in results:
            witness = item["witness"]
            moves = witness["zero_cc_trace"]["moves"] if witness else []
            writer.writerow(
                [
                    item["knot_id"],
                    item["catalogue_exact_u"],
                    item["search"]["status"],
                    item["search"]["attempts"],
                    item["standard_braid"]["strands"],
                    len(item["standard_braid"]["normalized_word"]),
                    item["standard_braid"]["rep_key"],
                    item["graph"]["node_id"],
                    item["graph"]["u_upper"],
                    ",".join(map(str, witness["crossing_change_positions"]))
                    if witness
                    else "",
                    sum(move["move"] == "RI" for move in moves),
                    sum(move["move"] == "RII" for move in moves),
                    sum(move["move"] == "RIII" for move in moves),
                ]
            )
    print(json.dumps(artifact["summary"], sort_keys=True))


if __name__ == "__main__":
    main()
