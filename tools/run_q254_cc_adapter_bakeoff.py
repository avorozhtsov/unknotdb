#!/usr/bin/env python3
"""Paired equal-budget L1000 MCTS bake-off for Q254 and its CC adapter."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import sqlite3
import struct
import time
from pathlib import Path

import torch

SCHEMA = "unknotdb-q254-cc-adapter-bakeoff-v0"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def atomic_json(path: Path, payload: object) -> None:
    temporary = path.with_name(f".{path.name}.{os.getpid()}.part")
    temporary.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    os.replace(temporary, path)


def minimal_rotation(word: list[int]) -> list[int]:
    if not word:
        return []
    return list(
        min(tuple(word[offset:] + word[:offset]) for offset in range(len(word)))
    )


def canonical_key(word: list[int], strands: int) -> bytes:
    writhe = sum(word)
    if writhe > 0:
        base = word
    elif writhe < 0:
        base = [-letter for letter in word]
    else:
        original = minimal_rotation(word)
        reflected = minimal_rotation([-letter for letter in word])
        base = [-letter for letter in word] if reflected < original else word
    canonical = minimal_rotation(base)
    encoded = b"UKB0" + struct.pack("<BBHI", 0, 0, strands, len(canonical))
    if canonical:
        encoded += struct.pack(f"<{len(canonical)}h", *canonical)
    return hashlib.sha256(encoded).digest()


def select_cohort(corpus: Path, graph: Path, limit: int) -> list[dict[str, object]]:
    connection = sqlite3.connect(f"file:{graph}?mode=ro", uri=True)
    rows = []
    seen = set()
    with corpus.open(newline="") as stream:
        for row in csv.DictReader(stream, delimiter="\t"):
            word = [int(value) for value in row["word"].split(",") if value]
            strands = int(row["strands"])
            if not (2 <= strands <= 12 and 3 <= len(word) <= 24):
                continue
            key = canonical_key(word, strands)
            found = connection.execute(
                "SELECT n.node_id,n.u_upper_bound FROM node_keys k "
                "JOIN nodes n USING(node_id) WHERE k.rep_key=? AND k.key_kind=0",
                (key,),
            ).fetchone()
            if found is None or not (1 <= int(found[1]) <= 5) or key in seen:
                continue
            seen.add(key)
            rows.append(
                {
                    "representation_id": row["representation_id"],
                    "strands": strands,
                    "word": word,
                    "word_length": len(word),
                    "graph_node": int(found[0]),
                    "graph_u": int(found[1]),
                    "roles": row["roles"],
                    "key": key.hex(),
                }
            )
    connection.close()
    # Round-robin U strata avoids a cohort dominated by easy U=1 diagrams.
    buckets = {upper: [] for upper in range(1, 6)}
    for row in sorted(
        rows,
        key=lambda item: (
            item["word_length"],
            item["strands"],
            item["representation_id"],
        ),
    ):
        buckets[row["graph_u"]].append(row)
    selected = []
    offset = 0
    while len(selected) < limit:
        added = False
        for upper in range(1, 6):
            if offset < len(buckets[upper]):
                selected.append(buckets[upper][offset])
                added = True
                if len(selected) == limit:
                    break
        if not added:
            break
        offset += 1
    return selected


def summarize(rows: list[dict], arm: str) -> dict[str, object]:
    attempts = [attempt for row in rows for attempt in row[arm]]
    solved = [attempt for attempt in attempts if attempt["solved"]]
    solved_items = sum(any(attempt["solved"] for attempt in row[arm]) for row in rows)
    return {
        "items": len(rows),
        "attempts": len(attempts),
        "solved_items": solved_items,
        "solved_attempts": len(solved),
        "solve_rate": len(solved) / max(len(attempts), 1),
        "mean_cc_solved": (
            sum(attempt["cc"] for attempt in solved) / len(solved) if solved else None
        ),
        "mean_moves_solved": (
            sum(attempt["moves"] for attempt in solved) / len(solved)
            if solved
            else None
        ),
        "wall_seconds": sum(attempt["wall_seconds"] for attempt in attempts),
        "attempts_at_or_below_graph_u": sum(
            attempt["solved"] and attempt["cc"] <= row["graph_u"]
            for row in rows
            for attempt in row[arm]
        ),
    }


def paired_summary(rows: list[dict]) -> dict[str, object]:
    outcomes = {
        "both_solved": 0,
        "baseline_only_solved": 0,
        "adapted_only_solved": 0,
        "neither_solved": 0,
        "baseline_lower_cc": 0,
        "adapted_lower_cc": 0,
        "equal_cc": 0,
        "baseline_lower_l1000": 0,
        "adapted_lower_l1000": 0,
        "equal_l1000": 0,
    }
    for row in rows:
        for baseline, adapted in zip(row["baseline"], row["adapted"], strict=True):
            if baseline["solved"] and adapted["solved"]:
                outcomes["both_solved"] += 1
                if baseline["cc"] < adapted["cc"]:
                    outcomes["baseline_lower_cc"] += 1
                elif adapted["cc"] < baseline["cc"]:
                    outcomes["adapted_lower_cc"] += 1
                else:
                    outcomes["equal_cc"] += 1
                if baseline["objective"] < adapted["objective"]:
                    outcomes["baseline_lower_l1000"] += 1
                elif adapted["objective"] < baseline["objective"]:
                    outcomes["adapted_lower_l1000"] += 1
                else:
                    outcomes["equal_l1000"] += 1
            elif baseline["solved"]:
                outcomes["baseline_only_solved"] += 1
            elif adapted["solved"]:
                outcomes["adapted_only_solved"] += 1
            else:
                outcomes["neither_solved"] += 1
    return outcomes


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--model-dir", type=Path, required=True)
    parser.add_argument("--adapter-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--pgx-root", type=Path, required=True)
    parser.add_argument("--rf-root", type=Path, required=True)
    parser.add_argument("--limit", type=int, default=64)
    parser.add_argument("--simulations", type=int, default=32)
    parser.add_argument("--attempts", type=int, default=2)
    parser.add_argument("--seed", type=int, default=20260829)
    args = parser.parse_args()
    if args.limit <= 0 or args.simulations <= 0 or args.attempts <= 0:
        raise ValueError("all budgets must be positive")

    import sys

    sys.path.insert(0, str((args.pgx_root / "src").resolve()))
    sys.path.insert(0, str((args.rf_root / "src").resolve()))
    from pgx_mcts_bench.adaptive_scientists import KnotItem, load_scientist
    from pgx_mcts_bench.collaboration_eval import _evaluation_record

    args.output_dir.mkdir(parents=True, exist_ok=True)
    model_manifest = json.loads((args.model_dir / "manifest.json").read_text())
    checkpoint = args.model_dir / model_manifest["checkpoint"]
    adapter_manifest = json.loads((args.adapter_dir / "manifest.json").read_text())
    adapter_path = args.adapter_dir / "adapter.pt"
    if sha256(checkpoint) != model_manifest["checkpoint_sha256"]:
        raise ValueError("Q254 hash mismatch")
    if sha256(adapter_path) != adapter_manifest["adapter"]["sha256"]:
        raise ValueError("adapter hash mismatch")
    if adapter_manifest["proof_policy_contract"]["checkpoint_sha256"] != sha256(
        checkpoint
    ):
        raise ValueError("adapter was trained against another Q254 checkpoint")

    cohort = select_cohort(args.corpus, args.graph, args.limit)
    if not cohort:
        raise ValueError("fixed cohort is empty")
    atomic_json(
        args.output_dir / "cohort.json",
        {
            "schema": f"{SCHEMA}-cohort",
            "selection": "round-robin graph U=1..5; then word_length,strands,representation_id",
            "source_corpus_sha256": sha256(args.corpus),
            "proof_graph_sha256": sha256(args.graph),
            "rows": cohort,
        },
    )
    scientist = load_scientist(
        model_manifest["scientist"],
        checkpoint,
        seed=args.seed,
        device="cpu",
        simulations=args.simulations,
        require_factorized=True,
        objective_budget_channel=True,
    )
    adapter = scientist.network.attach_option_policy_adapter()
    adapter.load_state_dict(
        torch.load(adapter_path, map_location="cpu", weights_only=False)["adapter"]
    )
    scientist.network.eval()
    torch.set_num_threads(min(8, os.cpu_count() or 1))

    results_path = args.output_dir / "results.json"
    completed = {
        row["representation_id"]: row
        for row in (
            json.loads(results_path.read_text()).get("rows", [])
            if results_path.exists()
            else []
        )
    }
    started = time.monotonic()
    for item_index, row in enumerate(cohort):
        identity = row["representation_id"]
        if identity in completed:
            continue
        knot = KnotItem(
            identity, row["word_length"], tuple(row["word"]), row["strands"]
        )
        result = {**row, "baseline": [], "adapted": []}
        for attempt in range(args.attempts):
            seed = args.seed + item_index * 10_000 + attempt
            for arm, enabled in (("baseline", False), ("adapted", True)):
                scientist.network.option_adapter_enabled = enabled
                verified, compute = _evaluation_record(
                    scientist,
                    knot,
                    1000.0,
                    args.simulations,
                    seed,
                    add_root_noise=True,
                )
                result[arm].append(
                    {
                        "seed": seed,
                        "solved": verified is not None,
                        "cc": verified[0] if verified is not None else None,
                        "moves": verified[1] if verified is not None else None,
                        "objective": (
                            1000 * verified[0] + verified[1]
                            if verified is not None
                            else None
                        ),
                        **compute,
                    }
                )
        completed[identity] = result
        ordered = [
            completed[item["representation_id"]]
            for item in cohort
            if item["representation_id"] in completed
        ]
        atomic_json(results_path, {"schema": f"{SCHEMA}-results", "rows": ordered})
        print(f"{len(ordered)}/{len(cohort)} {identity}", flush=True)

    rows = [completed[item["representation_id"]] for item in cohort]
    baseline = summarize(rows, "baseline")
    adapted = summarize(rows, "adapted")
    wins = {"baseline": 0, "adapted": 0, "ties": 0}
    for row in rows:
        left = min(
            (a["objective"] for a in row["baseline"] if a["solved"]), default=None
        )
        right = min(
            (a["objective"] for a in row["adapted"] if a["solved"]), default=None
        )
        if left is None and right is None or left == right:
            wins["ties"] += 1
        elif right is not None and (left is None or right < left):
            wins["adapted"] += 1
        else:
            wins["baseline"] += 1
    report = {
        "schema": SCHEMA,
        "status": "complete_not_published",
        "protocol": {
            "paired_same_seeds": True,
            "root_noise": True,
            "objective_ratio": 1000,
            "simulations_per_move": args.simulations,
            "attempts_per_item": args.attempts,
            "items": len(rows),
            "Q254_sha256": sha256(checkpoint),
            "adapter_sha256": sha256(adapter_path),
        },
        "baseline": baseline,
        "adapted": adapted,
        "paired_attempts": paired_summary(rows),
        "by_graph_u": {
            str(upper): {
                "baseline": summarize(
                    [row for row in rows if row["graph_u"] == upper], "baseline"
                ),
                "adapted": summarize(
                    [row for row in rows if row["graph_u"] == upper], "adapted"
                ),
            }
            for upper in range(1, 6)
        },
        "per_item_best_objective_wins": wins,
        "elapsed_seconds_this_invocation": time.monotonic() - started,
        "artifacts": {
            "cohort": str((args.output_dir / "cohort.json").resolve()),
            "results": str(results_path.resolve()),
        },
    }
    atomic_json(args.output_dir / "report.json", report)
    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
