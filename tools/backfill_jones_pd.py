#!/usr/bin/env python3
"""Backfill missing Jones polynomials by an exact PD-diagram state sum."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import re
import sqlite3
from pathlib import Path
from typing import Any

ALGORITHM = "pd-kauffman-state-sum-v1"


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def value_sha256(invariant_id: str, value_type: str, value: str) -> bytes:
    return hashlib.sha256(
        b"UNKNOTDB_INVARIANT_VALUE_V0\0"
        + invariant_id.encode()
        + b"\0"
        + value_type.encode()
        + b"\0"
        + value.encode()
    ).digest()


def load_representations(paths: list[Path]) -> dict[str, tuple[tuple[int, ...], int]]:
    result: dict[str, tuple[tuple[int, ...], int]] = {}
    for path in paths:
        for entry in json.loads(path.read_text())["entries"]:
            candidate = (tuple(map(int, entry["word"])), int(entry["strands"]))
            previous = result.setdefault(entry["representation_id"], candidate)
            if previous != candidate:
                raise ValueError(f"conflicting braid payload for {entry['representation_id']}")
    return result


def spherogram_name(canonical_name: str) -> str:
    if re.fullmatch(r"1[123][an]_\d+", canonical_name):
        return canonical_name.replace("_", "")
    return canonical_name


def pd_jones(link: Any) -> list[list[int]]:
    """Compute V(t) from a Spherogram PD code using exactly 2^crossings states."""
    pd = [tuple(map(int, crossing)) for crossing in link.PD_code()]
    labels = sorted({label for crossing in pd for label in crossing})
    label_index = {label: index for index, label in enumerate(labels)}
    bracket: dict[int, int] = {}
    crossing_count = len(pd)

    for state in range(1 << crossing_count):
        parent = list(range(len(labels)))

        def find(index: int, state_parent: list[int] = parent) -> int:
            while state_parent[index] != index:
                state_parent[index] = state_parent[state_parent[index]]
                index = state_parent[index]
            return index

        def union(left: int, right: int, state_parent: list[int] = parent) -> None:
            left_root = find(label_index[left])
            right_root = find(label_index[right])
            if left_root != right_root:
                state_parent[right_root] = left_root

        zero_smoothings = 0
        for crossing_index, (a, b, c, d) in enumerate(pd):
            if (state >> crossing_index) & 1:
                union(a, d)
                union(b, c)
            else:
                zero_smoothings += 1
                union(a, b)
                union(c, d)

        loops = len({find(index) for index in range(len(labels))})
        delta_power = loops - 1
        base_exponent = 2 * zero_smoothings - crossing_count
        delta_sign = -1 if delta_power % 2 else 1
        for positive_terms in range(delta_power + 1):
            exponent = base_exponent + 4 * positive_terms - 2 * delta_power
            coefficient = delta_sign * math.comb(delta_power, positive_terms)
            bracket[exponent] = bracket.get(exponent, 0) + coefficient

    writhe = sum(int(crossing.sign) for crossing in link.crossings)
    sign = -1 if abs(writhe) % 2 else 1
    normalized = {
        exponent - 3 * writhe: coefficient * sign
        for exponent, coefficient in bracket.items()
        if coefficient
    }
    if any(exponent % 4 for exponent in normalized):
        raise ValueError("normalized bracket has a non-integral t exponent")
    polynomial = {
        -exponent // 4: coefficient
        for exponent, coefficient in normalized.items()
        if coefficient
    }
    if sum(polynomial.values()) != 1:
        raise ValueError("Jones normalization check V(1)=1 failed")
    return [[exponent, polynomial[exponent]] for exponent in sorted(polynomial)]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--corpus-json", type=Path, action="append", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--expected-missing", type=int, default=531)
    parser.add_argument("--validate-known", action="store_true")
    args = parser.parse_args()
    if args.output.exists() or args.report.exists():
        raise FileExistsError("output artifact already exists")

    import spherogram  # type: ignore[import-not-found]

    representations = load_representations(args.corpus_json)
    source = sqlite3.connect(f"file:{args.input}?mode=ro", uri=True)
    source_meta = dict(source.execute("SELECT key,value FROM meta"))
    if source_meta["schema"] not in {
        "unknotdb-lookup-maps-v1",
        "unknotdb-lookup-maps-v2",
    }:
        raise ValueError("unsupported lookup-map schema")
    missing = list(
        source.execute(
            """
            SELECT k.knot_id,k.canonical_name,m.representation_id,d.value_text
            FROM knot_ids k
            JOIN representation_knot_map m USING(knot_id)
            JOIN knot_invariant_values d
              ON d.knot_id=k.knot_id AND d.invariant_id='determinant'
            WHERE NOT EXISTS(
                SELECT 1 FROM knot_invariant_values j
                WHERE j.knot_id=k.knot_id AND j.invariant_id='jones'
            )
            ORDER BY k.canonical_name,m.representation_id
            """
        )
    )
    if len(missing) != args.expected_missing:
        raise ValueError(f"expected {args.expected_missing} missing rows, found {len(missing)}")
    if len({row[0] for row in missing}) != len(missing):
        raise ValueError("backfill currently requires one source representation per missing knot")

    if args.validate_known:
        known = list(
            source.execute(
                """
                SELECT k.canonical_name,j.value_text FROM knot_ids k
                JOIN knot_invariant_values j USING(knot_id)
                WHERE j.invariant_id='jones' ORDER BY k.canonical_name
                """
            )
        )
        for index, (canonical_name, expected) in enumerate(known, 1):
            actual = json.dumps(
                pd_jones(spherogram.Link(spherogram_name(canonical_name))),
                separators=(",", ":"),
            )
            if actual != expected:
                raise ValueError(f"known Jones mismatch for {canonical_name}")
            if index % 500 == 0:
                print(f"validated-known {index}/{len(known)}", flush=True)
    else:
        known = []

    additions: list[tuple[str, str, str, str, bytes, str]] = []
    crossings: dict[int, int] = {}
    for index, (knot_id, canonical_name, representation_id, determinant_text) in enumerate(
        missing, 1
    ):
        link = spherogram.Link(spherogram_name(canonical_name))
        source_word = tuple(map(int, link.braid_word()))
        source_strands = max((abs(letter) for letter in source_word), default=0) + 1
        if representations.get(representation_id) != (source_word, source_strands):
            raise ValueError(f"source braid mismatch for {canonical_name}")
        pairs = pd_jones(link)
        determinant = abs(
            sum(coefficient * (-1) ** (exponent % 2) for exponent, coefficient in pairs)
        )
        if determinant != int(determinant_text):
            raise ValueError(f"Jones determinant check failed for {canonical_name}")
        value = json.dumps(pairs, separators=(",", ":"))
        additions.append(
            (
                knot_id,
                "jones",
                "laurent-pairs-json",
                value,
                value_sha256("jones", "laurent-pairs-json", value),
                (
                    "exact PD Kauffman state sum from the pinned Spherogram canonical "
                    "diagram; source braid identity checked exactly"
                ),
            )
        )
        crossings[len(link.crossings)] = crossings.get(len(link.crossings), 0) + 1
        if index % 50 == 0:
            print(f"computed-missing {index}/{len(missing)}", flush=True)

    temp = args.output.with_name(args.output.name + f".tmp-{os.getpid()}")
    destination = sqlite3.connect(temp)
    source.backup(destination)
    source.close()
    destination.executemany(
        "INSERT INTO knot_invariant_values VALUES (?,?,?,?,?,?)",
        additions,
    )
    metadata = {
        "schema": "unknotdb-lookup-maps-v2",
        "parent_lookup_maps_sha256": file_sha256(args.input),
        "jones_backfill_algorithm": ALGORITHM,
        "jones_backfill_count": str(len(additions)),
        "jones_backfill_spherogram_module": str(Path(spherogram.__file__).resolve()),
        "jones_backfill_corpus_sha256": json.dumps(
            [file_sha256(path) for path in args.corpus_json], separators=(",", ":")
        ),
    }
    destination.executemany(
        "INSERT OR REPLACE INTO meta VALUES (?,?)", sorted(metadata.items())
    )
    destination.execute("PRAGMA optimize")
    integrity = destination.execute("PRAGMA integrity_check").fetchone()[0]
    if integrity != "ok":
        raise RuntimeError(f"SQLite integrity failure: {integrity}")
    remaining = destination.execute(
        """
        SELECT count(*) FROM knot_ids k WHERE NOT EXISTS(
            SELECT 1 FROM knot_invariant_values j
            WHERE j.knot_id=k.knot_id AND j.invariant_id='jones'
        )
        """
    ).fetchone()[0]
    if remaining:
        raise RuntimeError(f"{remaining} knots still lack Jones values")
    destination.commit()
    destination.close()
    os.replace(temp, args.output)

    output_sha256 = file_sha256(args.output)
    report = f"""# Jones PD backfill

- Parent sidecar: `{args.input}`
- Parent SHA-256: `{metadata['parent_lookup_maps_sha256']}`
- Output sidecar: `{args.output}` ({args.output.stat().st_size:,} bytes)
- Output SHA-256: `{output_sha256}`
- Missing Jones values before: {len(missing):,}
- Missing Jones values after: 0
- Known values independently replayed: {len(known):,}
- Crossing-count distribution: `{json.dumps(crossings, sort_keys=True)}`
- Algorithm: `{ALGORITHM}`
- SQLite integrity: `ok`

Every source braid was checked for exact equality with the braid generated from the same pinned Spherogram named diagram. Each polynomial passed `V(1)=1` and `abs(V(-1))=det(K)`. The computation enumerates the minimal PD diagram's `2^crossings` Kauffman states rather than the wide braid's Temperley-Lieb basis.
"""
    args.report.write_text(report)
    print(
        f"backfilled={len(additions)} validated_known={len(known)} "
        f"remaining={remaining} bytes={args.output.stat().st_size} sha256={output_sha256}"
    )


if __name__ == "__main__":
    main()
