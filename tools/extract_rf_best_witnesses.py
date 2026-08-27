"""Extract the replay-verified RF/PGX best-solution pool into a compact TSV.

The source pool is read-only.  One row is emitted per semantic action so the
dependency-light Rust runtime can parse and independently replay every exact
checkpoint without adding a JSON dependency.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
from pathlib import Path
from typing import Any

SCHEMA = "unknotdb-rf-best-witness-steps-v0"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def word(value: Any) -> str:
    if not isinstance(value, list) or any(type(letter) is not int for letter in value):
        raise ValueError("witness word is not an integer list")
    return ",".join(str(letter) for letter in value)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pool", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    payload = json.loads(args.pool.read_text())
    if payload.get("schema") != "q-r-skm-evidence-catalog-v2":
        raise SystemExit("unsupported best-solution pool schema")
    verified = payload.get("verified", {}).get("best_by_representation")
    if not isinstance(verified, dict):
        raise SystemExit("best_by_representation is absent")

    fields = (
        "schema",
        "pool_sha256",
        "representation_id",
        "evidence_id",
        "l1000",
        "l10",
        "declared_cc",
        "declared_moves",
        "step_index",
        "start_strands",
        "start_word",
        "kind",
        "position",
        "generator",
        "sign",
        "after_strands",
        "after_word",
    )
    source_sha = sha256(args.pool)
    rows: list[dict[str, Any]] = []
    for representation_id, record in sorted(verified.items()):
        witness = record.get("witness")
        if not isinstance(witness, dict):
            raise TypeError(f"{representation_id}: witness is absent")
        start = witness.get("start")
        steps = witness.get("steps")
        if not isinstance(start, dict) or not isinstance(steps, list) or not steps:
            raise ValueError(f"{representation_id}: incomplete witness")
        for index, step in enumerate(steps):
            action = step.get("action")
            after = step.get("after")
            if not isinstance(action, dict) or not isinstance(after, dict):
                raise TypeError(f"{representation_id}: malformed step {index}")
            rows.append(
                {
                    "schema": SCHEMA,
                    "pool_sha256": source_sha,
                    "representation_id": representation_id,
                    "evidence_id": record["evidence_id"],
                    "l1000": record["l1000"],
                    "l10": record["l10"],
                    "declared_cc": witness["crossing_changes"],
                    "declared_moves": witness["moves"],
                    "step_index": index,
                    "start_strands": start["strands"],
                    "start_word": word(start["word"]),
                    "kind": action["kind"],
                    "position": action.get("position", ""),
                    "generator": action.get("generator", ""),
                    "sign": action.get("sign", ""),
                    "after_strands": after["strands"],
                    "after_word": word(after["word"]),
                }
            )

    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", encoding="utf-8", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=fields, delimiter="\t", lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)
    print(f"witnesses={len(verified)} steps={len(rows)} pool_sha256={source_sha}")


if __name__ == "__main__":
    main()
