#!/usr/bin/env python3
"""Build the deterministic unresolved DGKT braid cohort for frozen-policy MCTS."""

from __future__ import annotations

import argparse
import hashlib
import json
import sqlite3
from pathlib import Path

from spherogram import Link


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--lower-bounds", type=Path, required=True)
    parser.add_argument("--exact-list", type=Path, required=True)
    parser.add_argument("--range-list", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--target-tsv", type=Path)
    parser.add_argument("--target-limit", type=int)
    args = parser.parse_args()

    kinds: dict[str, str] = {}
    for path, kind in ((args.exact_list, "exact"), (args.range_list, "range")):
        for line in path.read_text().splitlines():
            knot_id = line.strip()
            if knot_id and not knot_id.startswith("#"):
                kinds[knot_id] = kind

    db = sqlite3.connect(f"file:{args.lower_bounds}?mode=ro", uri=True)
    rows = []
    rejected = []
    for knot_id, interval_kind in sorted(kinds.items()):
        claim = db.execute(
            "SELECT crossing_number,u_lower,retained_upper,interval_exact "
            "FROM lower_bound_claims WHERE knot_id=?",
            (knot_id,),
        ).fetchone()
        if claim is None:
            rejected.append({"knot_id": knot_id, "reason": "missing-lower-bound-row"})
            continue
        name = knot_id.removeprefix("knot:")
        try:
            word = [int(letter) for letter in Link(name.replace("_", "")).braid_word()]
        except Exception as error:  # noqa: BLE001 - external catalogue parser
            rejected.append({"knot_id": knot_id, "reason": f"spherogram:{error}"})
            continue
        strands = max((abs(letter) for letter in word), default=0) + 1
        compatible = strands <= 12 and len(word) <= 48
        rows.append(
            {
                "knot_id": knot_id,
                "name": name,
                "interval_kind": interval_kind,
                "crossing_number": int(claim[0]),
                "u_lower": int(claim[1]),
                "target_upper": int(claim[2]),
                "interval_exact": bool(claim[3]),
                "strands": strands,
                "word": word,
                "word_length": len(word),
                "q254_compatible": compatible,
                "incompatibility": None if compatible else "Q254 strands<=12,length<=48",
            }
        )
    db.close()
    rows.sort(
        key=lambda row: (
            not row["q254_compatible"],
            row["target_upper"],
            row["word_length"],
            row["strands"],
            row["knot_id"],
        )
    )
    payload = {
        "schema": "unknotdb-dgkt-q254-mcts-cohort-v0",
        "inputs": {
            "lower_bounds": str(args.lower_bounds.resolve()),
            "lower_bounds_sha256": sha256(args.lower_bounds),
            "exact_list": str(args.exact_list.resolve()),
            "exact_list_sha256": sha256(args.exact_list),
            "range_list": str(args.range_list.resolve()),
            "range_list_sha256": sha256(args.range_list),
            "braid_source": "Spherogram Link(name).braid_word()",
        },
        "summary": {
            "selected": len(rows),
            "q254_compatible": sum(row["q254_compatible"] for row in rows),
            "q254_incompatible": sum(not row["q254_compatible"] for row in rows),
            "rejected": len(rejected),
        },
        "rows": rows,
        "rejected": rejected,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    temporary = args.output.with_suffix(args.output.suffix + ".tmp")
    temporary.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    temporary.replace(args.output)
    if args.target_tsv is not None:
        compatible = [row for row in rows if row["q254_compatible"]]
        if args.target_limit is not None:
            if args.target_limit <= 0:
                raise ValueError("--target-limit must be positive")
            compatible = compatible[: args.target_limit]
        lines = ["representation_id\ttarget_u\tstrands\tword"]
        lines.extend(
            "\t".join(
                (
                    str(row["knot_id"]),
                    str(row["target_upper"]),
                    str(row["strands"]),
                    ",".join(str(letter) for letter in row["word"]),
                )
            )
            for row in compatible
        )
        args.target_tsv.parent.mkdir(parents=True, exist_ok=True)
        target_temporary = args.target_tsv.with_suffix(args.target_tsv.suffix + ".tmp")
        target_temporary.write_text("\n".join(lines) + "\n")
        target_temporary.replace(args.target_tsv)
    print(json.dumps(payload["summary"], sort_keys=True))


if __name__ == "__main__":
    main()
