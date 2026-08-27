#!/usr/bin/env python3
"""Build the deterministic RF-sourced optimizer cohort from an audit manifest."""

from __future__ import annotations

import argparse
import csv
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--audit", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--selection-manifest", required=True, type=Path)
    parser.add_argument("--limit", type=int, default=2000)
    args = parser.parse_args()

    with args.corpus.open(newline="", encoding="utf-8") as handle:
        corpus_rows = list(csv.DictReader(handle, delimiter="\t"))
    by_id = {row["representation_id"]: row for row in corpus_rows}

    audited: list[dict[str, str]] = []
    with args.audit.open(encoding="utf-8") as handle:
        for line in handle:
            fields = line.rstrip("\n").split("\t")
            if len(fields) == 7 and fields[0] == "audit" and fields[1] != "representation_id":
                audited.append(
                    {
                        "representation_id": fields[1],
                        "priority": fields[2],
                        "stop_reason": fields[3],
                        "stopping_key": fields[4],
                        "covered": fields[5],
                        "old_u": fields[6],
                    }
                )

    def rank(row: dict[str, str]) -> tuple[int, int, int, str, str]:
        covered = row["covered"] == "1"
        old_u = int(row["old_u"]) if row["old_u"] != "-" else -1
        return (
            0 if covered else 1,
            -old_u,
            int(row["priority"]),
            row["representation_id"],
            row["stopping_key"],
        )

    selected = sorted(audited, key=rank)[: args.limit]
    if len(selected) != args.limit:
        raise SystemExit(f"requested {args.limit} rows but only {len(selected)} audited rows exist")

    corpus_header = ["representation_id", "priority", "strands", "word", "roles"]
    selection_header = [
        "ordinal",
        "representation_id",
        "priority",
        "stopping_key",
        "covered",
        "old_u",
        "stop_reason",
    ]
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=corpus_header, delimiter="\t", lineterminator="\n")
        writer.writeheader()
        for audit_row in selected:
            writer.writerow(by_id[audit_row["representation_id"]])
    with args.selection_manifest.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=selection_header, delimiter="\t", lineterminator="\n")
        writer.writeheader()
        for ordinal, row in enumerate(selected, 1):
            writer.writerow({"ordinal": ordinal, **row})

    covered = sum(row["covered"] == "1" for row in selected)
    print(f"selected={len(selected)} covered={covered} disconnected={len(selected) - covered}")


if __name__ == "__main__":
    main()
