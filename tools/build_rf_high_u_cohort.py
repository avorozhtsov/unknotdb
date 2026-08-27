#!/usr/bin/env python3
"""Build a deterministic canonical-key cohort from an RF coverage audit."""

from __future__ import annotations

import argparse
import csv
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--audit", type=Path, required=True)
    parser.add_argument("--keys", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--min-u", type=int, default=11)
    args = parser.parse_args()

    if args.keys.exists() or args.manifest.exists():
        raise FileExistsError("cohort outputs must not already exist")

    selected: dict[str, tuple[int, list[str]]] = {}
    with args.audit.open(newline="", encoding="utf-8") as source:
        for row in csv.reader(source, delimiter="\t"):
            if len(row) < 8 or row[0] != "representation" or row[1] == "id":
                continue
            representation_id, key, new_u, status = row[1], row[2], row[4], row[7]
            if status != "already-covered" or key == "-" or new_u == "-":
                continue
            u = int(new_u)
            if u < args.min_u:
                continue
            previous = selected.get(key)
            if previous is None:
                selected[key] = (u, [representation_id])
            else:
                if previous[0] != u:
                    raise ValueError(f"canonical key {key} has inconsistent U values")
                previous[1].append(representation_id)

    ordered = sorted(selected.items(), key=lambda item: (-item[1][0], item[0]))
    args.keys.write_text("".join(f"{key}\n" for key, _ in ordered), encoding="utf-8")
    with args.manifest.open("w", newline="", encoding="utf-8") as target:
        writer = csv.writer(target, delimiter="\t", lineterminator="\n")
        writer.writerow(["unknotdb-rf-high-u-cohort-v0"])
        writer.writerow(["source_audit", args.audit])
        writer.writerow(["min_u", args.min_u])
        writer.writerow(["ordinal", "stopping_key", "u_upper", "rf_representations"])
        for ordinal, (key, (u, representations)) in enumerate(ordered):
            writer.writerow([ordinal, key, u, ",".join(sorted(representations))])
        writer.writerow(["summary", len(ordered)])
    print(f"published RF high-U cohort: {len(ordered)} canonical keys")


if __name__ == "__main__":
    main()
