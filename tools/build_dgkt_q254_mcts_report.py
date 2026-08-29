#!/usr/bin/env python3
"""Summarize a completed frozen-Q254 DGKT graph-MCTS campaign."""

from __future__ import annotations

import argparse
import hashlib
import json
import statistics
from collections import Counter
from pathlib import Path


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def percentile(values: list[float], fraction: float) -> float | None:
    if not values:
        return None
    return sorted(values)[round((len(values) - 1) * fraction)]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--run-dir", type=Path, required=True)
    parser.add_argument("--cohort", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    manifest_path = args.run_dir / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    cohort = json.loads(args.cohort.read_text())
    results = [json.loads(path.read_text()) for path in sorted((args.run_dir / "items").glob("*.json"))]
    attempts = [result["attempts"][0] for result in results]
    completed_routes = [
        {"knot_id": result["knot_id"], **attempt}
        for result, attempt in zip(results, attempts, strict=True)
        if attempt["termination_reason"] == "solved"
    ]
    elapsed = [float(attempt["elapsed_seconds"]) for attempt in attempts]
    used_cc = [int(attempt["route_cc"]) for attempt in attempts]
    incompatible = [
        {
            "knot_id": row["knot_id"],
            "strands": row["strands"],
            "word_length": row["word_length"],
            "reason": row["incompatibility"],
        }
        for row in cohort["rows"]
        if not row["q254_compatible"]
    ]
    report = {
        "schema": "unknotdb-dgkt-q254-graph-mcts-report-v0",
        "contract": {
            "policy": manifest["model_id"],
            "checkpoint_sha256": manifest["checkpoint_sha256"],
            "objective": "L1000",
            "graph_terminal": "exact mirror-orbit key after raw normalization or deterministic decreasing RI/RII",
            "acceptance": "route_cc + graph_target_u <= DGKT retained_upper",
            "proof_publication": "none; every accepted trace would still require Rust semantic replay/import",
            "training_performed": False,
        },
        "inputs": {
            "graph": manifest["graph"],
            "graph_sha256": manifest["graph_sha256"],
            "cohort": str(args.cohort.resolve()),
            "cohort_sha256": sha256(args.cohort),
            "run_manifest": str(manifest_path.resolve()),
            "run_manifest_sha256": sha256(manifest_path),
        },
        "budget": manifest["budgets"],
        "coverage": {
            "requested": len(cohort["rows"]),
            "q254_compatible": len(results),
            "q254_capacity_misses": len(incompatible),
            "completed_attempts": len(attempts),
            "accepted_target_hits": sum(bool(result["success"]) for result in results),
            "graph_hits": sum(bool(attempt["graph_hit"]) for attempt in attempts),
            "ordinary_unknot_terminations": len(completed_routes),
            "move_budget_exhaustions": sum(
                attempt["termination_reason"] == "move_budget_exhausted" for attempt in attempts
            ),
        },
        "observed_effort": {
            "wall_seconds": manifest["elapsed_seconds"],
            "per_item_seconds_p50": statistics.median(elapsed),
            "per_item_seconds_p95": percentile(elapsed, 0.95),
            "cc_spent_before_termination_p50": statistics.median(used_cc),
            "cc_spent_before_termination_p95": percentile(used_cc, 0.95),
            "cc_spent_before_termination_max": max(used_cc),
            "termination_reasons": dict(sorted(Counter(a["termination_reason"] for a in attempts).items())),
        },
        "completed_but_rejected_routes": completed_routes,
        "capacity_misses": incompatible,
        "result": {
            "new_nodes": 0,
            "new_edges": 0,
            "new_programs": 0,
            "u_improvements": 0,
            "published_snapshot": None,
            "conclusion": "Frozen Q254 MCTS did not meet any DGKT target in this bounded cohort.",
        },
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    temporary = args.output.with_suffix(args.output.suffix + ".tmp")
    temporary.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    temporary.replace(args.output)
    print(json.dumps(report["coverage"], sort_keys=True))


if __name__ == "__main__":
    main()
