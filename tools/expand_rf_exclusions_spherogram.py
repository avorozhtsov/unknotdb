#!/usr/bin/env python3
"""Generate braid words for RF corpus exclusions using installed Spherogram."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
from collections import Counter
from pathlib import Path


def representation_id(word: tuple[int, ...], strands: int) -> str:
    payload = json.dumps(
        {"encoding": "braid-word-v1", "strands": strands, "word": list(word)},
        sort_keys=True,
        separators=(",", ":"),
    ).encode()
    return "braid:" + hashlib.sha256(payload).hexdigest()


def component_count(word: tuple[int, ...], strands: int) -> int:
    permutation = list(range(strands))
    for letter in word:
        index = abs(letter) - 1
        permutation[index], permutation[index + 1] = permutation[index + 1], permutation[index]
    seen: set[int] = set()
    cycles = 0
    for start in range(strands):
        if start in seen:
            continue
        cycles += 1
        current = start
        while current not in seen:
            seen.add(current)
            current = permutation[current]
    return cycles


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--corpus-json", type=Path, required=True)
    parser.add_argument("--output-tsv", type=Path, required=True)
    parser.add_argument("--output-json", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--max-policy-strands", type=int, default=12)
    parser.add_argument("--max-policy-word-length", type=int, default=48)
    args = parser.parse_args()
    for path in (args.output_tsv, args.output_json, args.report):
        if path.exists():
            raise FileExistsError(path)

    import spherogram  # type: ignore[import-not-found]

    source = json.loads(args.corpus_json.read_text())
    entries: dict[str, dict[str, object]] = {}
    failures: list[dict[str, str]] = []
    reasons = Counter()
    for exclusion in source["exclusions"]:
        reason = exclusion["reason"]
        if reason == "closure is not a single-component knot":
            failures.append({**exclusion, "expansion_status": "excluded-link"})
            reasons["excluded-link"] += 1
            continue
        source_id = exclusion.get("source_id")
        if not source_id:
            failures.append({**exclusion, "expansion_status": "missing-source-id"})
            reasons["missing-source-id"] += 1
            continue
        lookup_name = str(source_id).replace("_", "")
        try:
            link = spherogram.Link(lookup_name)
            word = tuple(int(letter) for letter in link.braid_word())
            strands = max((abs(letter) for letter in word), default=0) + 1
            if len(link.link_components) != 1 or component_count(word, strands) != 1:
                raise ValueError("generated closure is not one component")
        except Exception as error:  # noqa: BLE001 - catalogue boundary
            failures.append(
                {**exclusion, "expansion_status": f"spherogram-error:{type(error).__name__}:{error}"}
            )
            reasons[f"spherogram-error:{type(error).__name__}"] += 1
            continue
        identity = representation_id(word, strands)
        compatible = strands <= args.max_policy_strands and len(word) <= args.max_policy_word_length
        entry = entries.setdefault(
            identity,
            {
                "representation_id": identity,
                "priority": 3,
                "strands": strands,
                "word": list(word),
                "word_length": len(word),
                "roles": ["rf-exclusion-spherogram-expansion"],
                "policy_compatible": compatible,
                "source_refs": [],
            },
        )
        entry["source_refs"].append(
            {
                "source_id": source_id,
                "lookup_name": lookup_name,
                "path": exclusion["path"],
                "pointer": exclusion["pointer"],
                "role": exclusion["role"],
                "original_exclusion_reason": reason,
            }
        )
        reasons["generated-policy-compatible" if compatible else "generated-outside-policy-capacity"] += 1

    ordered = sorted(
        entries.values(),
        key=lambda entry: (
            not entry["policy_compatible"],
            entry["strands"],
            entry["word_length"],
            entry["representation_id"],
        ),
    )
    with args.output_tsv.open("w", newline="") as stream:
        writer = csv.writer(stream, delimiter="\t", lineterminator="\n")
        writer.writerow(["representation_id", "priority", "strands", "word", "roles"])
        for entry in ordered:
            if not entry["policy_compatible"]:
                continue
            writer.writerow(
                [
                    entry["representation_id"],
                    entry["priority"],
                    entry["strands"],
                    ",".join(map(str, entry["word"])),
                    ",".join(entry["roles"]),
                ]
            )
    document = {
        "schema": "unknotdb-rf-spherogram-expansion-v0",
        "source_corpus_sha256": hashlib.sha256(args.corpus_json.read_bytes()).hexdigest(),
        "spherogram_module": str(Path(spherogram.__file__).resolve()),
        "policy_capacity": {
            "max_strands": args.max_policy_strands,
            "max_word_length": args.max_policy_word_length,
        },
        "summary": dict(sorted(reasons.items())),
        "entries": ordered,
        "failures": failures,
    }
    args.output_json.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n")
    compatible_count = sum(bool(entry["policy_compatible"]) for entry in ordered)
    report = f"""# RF exclusion braid expansion

- Original exclusions: {len(source['exclusions']):,}
- Generated unique braid representations: {len(ordered):,}
- Compatible with frozen Q254 capacity (`strands≤{args.max_policy_strands}`, `length≤{args.max_policy_word_length}`): {compatible_count:,}
- Generated but outside policy capacity: {len(ordered) - compatible_count:,}
- Unresolved/excluded source records: {len(failures):,}
- Spherogram module: `{Path(spherogram.__file__).resolve()}`

Every generated word was checked to have a one-component braid closure. Spherogram is used only as a representation builder; graph admission still requires pinned preprocessing and independently replayed descending or policy proof edges.
"""
    args.report.write_text(report)
    print(
        f"generated={len(ordered)} compatible={compatible_count} "
        f"outside_capacity={len(ordered)-compatible_count} failures={len(failures)}"
    )


if __name__ == "__main__":
    main()
