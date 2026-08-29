#!/usr/bin/env python3
"""Run resumable, timeout-bounded DGKT upper-witness reconstruction."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

FORMAT = "unknotdb-dgkt-upper-witness-campaign-v0"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def append_jsonl(path: Path, row: dict[str, object]) -> None:
    with path.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n")
        handle.flush()
        os.fsync(handle.fileno())


def atomic_json(path: Path, value: object) -> None:
    temporary = path.with_name(f"{path.name}.tmp-{os.getpid()}")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
    os.replace(temporary, path)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--catalogue", type=Path, required=True)
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--crossings", type=int, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--combined-output", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--per-knot-timeout", type=float, default=60.0)
    parser.add_argument("--worker-python", type=Path, default=Path(sys.executable))
    parser.add_argument("--max-type-iii", type=int, default=250)
    parser.add_argument("--seeded-fallback-seeds", type=int, default=4)
    parser.add_argument("--limit", type=int)
    parser.add_argument("--shard-count", type=int, default=1)
    parser.add_argument("--shard-index", type=int, default=0)
    parser.add_argument(
        "--interval-kind", choices=("exact", "range", "all"), default="exact"
    )
    parser.add_argument("--knot-list", type=Path)
    args = parser.parse_args()
    if args.per_knot_timeout <= 0:
        raise ValueError("--per-knot-timeout must be positive")
    if args.shard_count <= 0 or not 0 <= args.shard_index < args.shard_count:
        raise ValueError("shard index must be in [0, shard count)")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    args.manifest.parent.mkdir(parents=True, exist_ok=True)
    args.combined_output.parent.mkdir(parents=True, exist_ok=True)

    catalogue = sqlite3.connect(f"file:{args.catalogue}?mode=ro", uri=True)
    interval_clause = {
        "exact": "interval_exact=1",
        "range": "interval_exact=0",
        "all": "1=1",
    }[args.interval_kind]
    knots = [
        row[0]
        for row in catalogue.execute(
            f"""
            SELECT knot_id FROM lower_bound_claims
            WHERE {interval_clause} AND crossing_number=? ORDER BY knot_id
            """,
            (args.crossings,),
        )
    ]
    catalogue.close()
    if args.knot_list:
        wanted = {
            line.strip()
            for line in args.knot_list.read_text().splitlines()
            if line.strip() and not line.startswith("#")
        }
        knots = [knot for knot in knots if knot in wanted]
    if args.limit is not None:
        if args.limit <= 0:
            raise ValueError("--limit must be positive")
        knots = knots[: args.limit]
    knots = knots[args.shard_index :: args.shard_count]

    tool = Path(__file__).with_name("build_catalogue_reidemeister_witnesses.py")
    records: list[dict[str, object]] = []
    started = time.monotonic()
    for index, knot_id in enumerate(knots, 1):
        safe_name = knot_id.replace(":", "-")
        artifact = args.output_dir / f"{safe_name}.json"
        summary = args.output_dir / f"{safe_name}.tsv"
        if artifact.exists():
            payload = json.loads(artifact.read_text())
            result = payload["results"][0]
            status = str(result["search"]["status"])
            records.append(result)
            print(f"{index}/{len(knots)}\t{knot_id}\tresume-{status}", flush=True)
            continue
        command = [
            str(args.worker_python),
            str(tool),
            "--catalogue",
            str(args.catalogue),
            "--graph",
            str(args.graph),
            "--min-crossings",
            str(args.crossings),
            "--max-crossings",
            str(args.crossings),
            "--max-type-iii",
            str(args.max_type_iii),
            "--seeded-fallback-seeds",
            str(args.seeded_fallback_seeds),
            "--only-knot",
            knot_id,
            "--output",
            str(artifact),
            "--summary",
            str(summary),
        ]
        if args.interval_kind != "exact":
            command.append("--include-ranges")
        item_started = time.monotonic()
        try:
            completed = subprocess.run(
                command,
                text=True,
                capture_output=True,
                timeout=args.per_knot_timeout,
                check=False,
            )
            elapsed = time.monotonic() - item_started
            if completed.returncode != 0:
                status = "error"
                detail = completed.stderr[-1000:]
            else:
                payload = json.loads(artifact.read_text())
                result = payload["results"][0]
                records.append(result)
                status = str(result["search"]["status"])
                detail = completed.stdout[-1000:]
        except subprocess.TimeoutExpired:
            elapsed = time.monotonic() - item_started
            status = "timeout"
            detail = f"exceeded {args.per_knot_timeout} seconds"
        row = {
            "index": index,
            "knot_id": knot_id,
            "status": status,
            "elapsed_seconds": elapsed,
            "detail": detail,
        }
        append_jsonl(args.manifest, row)
        print(f"{index}/{len(knots)}\t{knot_id}\t{status}\t{elapsed:.3f}s", flush=True)

    witnesses = sum(result["search"]["status"] == "witness" for result in records)
    combined = {
        "format": "unknotdb-catalogue-reidemeister-witness-cohort-v0",
        "campaign_format": FORMAT,
        "inputs": {
            "catalogue": str(args.catalogue),
            "catalogue_sha256": sha256(args.catalogue),
            "graph": str(args.graph),
            "graph_sha256": sha256(args.graph),
            "crossings": args.crossings,
            "per_knot_timeout": args.per_knot_timeout,
            "max_type_iii": args.max_type_iii,
            "seeded_fallback_seeds": args.seeded_fallback_seeds,
            "worker_python": str(args.worker_python),
            "shard_count": args.shard_count,
            "shard_index": args.shard_index,
            "interval_kind": args.interval_kind,
            "knot_list": str(args.knot_list) if args.knot_list else None,
        },
        "summary": {
            "selected": len(knots),
            "completed_artifacts": len(records),
            "witnesses": witnesses,
            "misses": len(records) - witnesses,
            "unresolved": len(knots) - len(records),
            "elapsed_seconds": time.monotonic() - started,
        },
        "results": records,
    }
    atomic_json(args.combined_output, combined)
    print(json.dumps(combined["summary"], sort_keys=True))


if __name__ == "__main__":
    main()
