#!/usr/bin/env python3
"""Build the protected hard-case benchmark sidecar for global embeddings."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sqlite3
from pathlib import Path

SCHEMA = "unknotdb-embedding-hard-cases-v0"
CASE_ID = "wang-zhang:7_1-sum-mirror-7_1:u5-cube"
LONG_CASE_ID = "proof-graph:exact-cc01-long-witnesses:protected"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def load_states(path: Path) -> list[tuple[int, str, int]]:
    states = []
    for line in path.read_text().splitlines():
        fields = line.split("\t")
        if len(fields) == 6 and fields[0] == "state" and fields[1] != "mask":
            mask = int(fields[1], 16)
            key = fields[3]
            u_upper = int(fields[4])
            states.append((mask, key, u_upper))
    if len(states) != 32 or {mask for mask, _, _ in states} != set(range(32)):
        raise ValueError("expected the complete 32-state five-crossing cube")
    for mask, _, u_upper in states:
        if u_upper != 5 - mask.bit_count():
            raise ValueError(f"state {mask:02x} has inconsistent U={u_upper}")
    return sorted(states)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pairs", type=Path, required=True)
    parser.add_argument("--cube-manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    states = load_states(args.cube_manifest)
    source = sqlite3.connect(f"file:{args.pairs}?mode=ro", uri=True)
    mapped = {}
    for mask, key, u_upper in states:
        row = source.execute(
            "SELECT representation_id,strands,word_length,split "
            "FROM representations WHERE lower(hex(sha256))=?",
            (key,),
        ).fetchone()
        if row is None:
            raise ValueError(f"cube state {mask:02x} key {key} is absent")
        mapped[mask] = (int(row[0]), int(row[1]), int(row[2]), str(row[3]), key)

    def direct_witness_length(left_mask: int, right_mask: int) -> int:
        left_id, right_id = sorted((mapped[left_mask][0], mapped[right_mask][0]))
        row = source.execute(
            "SELECT MIN(witness_semantic_length) FROM ("
            "SELECT left_representation,right_representation,distance_upper,"
            "witness_semantic_length FROM pairs WHERE metric='cc' UNION ALL "
            "SELECT left_representation,right_representation,distance_upper,"
            "witness_semantic_length FROM excluded_cross_split_pairs "
            "WHERE metric='cc') WHERE left_representation=? "
            "AND right_representation=? AND distance_upper<=1",
            (left_id, right_id),
        ).fetchone()
        if row is None or row[0] is None:
            raise ValueError(
                f"cube edge {left_mask:02x}->{right_mask:02x} has no CC<=1 witness"
            )
        return int(row[0])

    edge_lengths = {}
    for left_mask in range(32):
        for bit in range(5):
            if left_mask & (1 << bit):
                continue
            right_mask = left_mask | (1 << bit)
            edge_lengths[(left_mask, right_mask)] = direct_witness_length(
                left_mask, right_mask
            )
    long_candidates = list(
        source.execute(
            "SELECT left_representation,right_representation,distance_lower,"
            "distance_upper,witness_semantic_length,provenance FROM pairs "
            "WHERE split='train' AND metric='cc' AND distance_lower=distance_upper "
            "AND distance_upper<=1 AND witness_semantic_length>5 "
            "ORDER BY witness_semantic_length DESC,left_representation,right_representation"
        )
    )
    protected_long = [
        row
        for row in long_candidates
        if hashlib.sha256(f"{row[0]}:{row[1]}".encode()).digest()[0] % 5 == 0
    ]

    temporary = args.output.with_name(f".{args.output.name}.{os.getpid()}.part")
    temporary.unlink(missing_ok=True)
    target = sqlite3.connect(temporary)
    target.executescript(
        """
        PRAGMA foreign_keys=ON;
        CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;
        CREATE TABLE hard_cases(
          case_id TEXT PRIMARY KEY,
          label_status TEXT NOT NULL,
          source TEXT NOT NULL,
          rationale TEXT NOT NULL,
          protected_split TEXT NOT NULL,
          structure_kind TEXT NOT NULL
        ) WITHOUT ROWID;
        CREATE TABLE hard_case_representations(
          case_id TEXT NOT NULL REFERENCES hard_cases(case_id),
          representation_id INTEGER NOT NULL,
          representation_sha256 TEXT NOT NULL CHECK(length(representation_sha256)=64),
          state_label TEXT NOT NULL,
          cc_from_source INTEGER NOT NULL,
          cc_to_terminal INTEGER NOT NULL,
          strands INTEGER NOT NULL,
          word_length INTEGER NOT NULL,
          original_split TEXT NOT NULL,
          PRIMARY KEY(case_id,representation_id)
        ) WITHOUT ROWID;
        CREATE TABLE hard_case_pairs(
          case_id TEXT NOT NULL REFERENCES hard_cases(case_id),
          left_representation INTEGER NOT NULL,
          right_representation INTEGER NOT NULL,
          metric TEXT NOT NULL CHECK(metric='cc'),
          distance_lower INTEGER NOT NULL,
          distance_upper INTEGER NOT NULL,
          witness_semantic_length INTEGER NOT NULL,
          label_status TEXT NOT NULL,
          derivation TEXT NOT NULL,
          PRIMARY KEY(case_id,left_representation,right_representation,metric)
        ) WITHOUT ROWID;
        """
    )
    metadata = {
        "schema": SCHEMA,
        "pairs_sha256": sha256(args.pairs),
        "cube_manifest_sha256": sha256(args.cube_manifest),
        "case_count": "2",
        "distance_scope": "cc_1_2",
    }
    target.executemany("INSERT INTO meta VALUES (?,?)", metadata.items())
    target.execute(
        "INSERT INTO hard_cases VALUES (?,?,?,?,?,?)",
        (
            CASE_ID,
            "externally_attested_exact_geodesic",
            "Wang--Zhang arXiv:2507.14265; replayed five-CC cube manifest",
            (
                "The cited lower bound u=5 and the replayed five-edge "
                "source-to-unknot path force every comparable Boolean-cube "
                "subpath to be geodesic."
            ),
            "protected_test",
            "boolean_geodesic_cube",
        ),
    )
    for mask, (representation_id, strands, length, split, key) in mapped.items():
        target.execute(
            "INSERT INTO hard_case_representations VALUES (?,?,?,?,?,?,?,?,?)",
            (
                CASE_ID,
                representation_id,
                key,
                f"mask-{mask:02x}",
                mask.bit_count(),
                5 - mask.bit_count(),
                strands,
                length,
                split,
            ),
        )

    pair_count = {1: 0, 2: 0}
    for left_mask in range(32):
        for right_mask in range(left_mask + 1, 32):
            if left_mask & right_mask not in (left_mask, right_mask):
                continue
            distance = (left_mask ^ right_mask).bit_count()
            if distance not in pair_count:
                continue
            lower_mask, upper_mask = (
                (left_mask, right_mask)
                if left_mask & right_mask == left_mask
                else (right_mask, left_mask)
            )
            if distance == 1:
                witness_length = edge_lengths[(lower_mask, upper_mask)]
            else:
                added = upper_mask ^ lower_mask
                intermediates = [
                    lower_mask | (1 << bit)
                    for bit in range(5)
                    if added & (1 << bit)
                ]
                witness_length = min(
                    edge_lengths[(lower_mask, middle)]
                    + edge_lengths[(middle, upper_mask)]
                    for middle in intermediates
                )
            left_id = mapped[left_mask][0]
            right_id = mapped[right_mask][0]
            left_id, right_id = sorted((left_id, right_id))
            target.execute(
                "INSERT INTO hard_case_pairs VALUES (?,?,?,?,?,?,?,?,?)",
                (
                    CASE_ID,
                    left_id,
                    right_id,
                    "cc",
                    distance,
                    distance,
                    witness_length,
                    "externally_attested_exact_geodesic",
                    (
                        "comparable cube states; otherwise a shorter subpath would "
                        "contradict the cited source-to-unknot CC lower bound 5"
                    ),
                ),
            )
            pair_count[distance] += 1

    target.execute(
        "INSERT INTO hard_cases VALUES (?,?,?,?,?,?)",
        (
            LONG_CASE_ID,
            "replay_plus_attested_exact",
            "UnknotDB immutable proof programs and attested endpoint identities",
            (
                "Deterministic 20 percent representation-level holdout of exact "
                "CC=0/1 pairs whose shortest stored semantic witness exceeds five."
            ),
            "protected_test",
            "pair_cohort",
        ),
    )
    long_representation_ids = sorted(
        {int(row[0]) for row in protected_long}
        | {int(row[1]) for row in protected_long}
    )
    for representation_id in long_representation_ids:
        row = source.execute(
            "SELECT lower(hex(sha256)),strands,word_length,split FROM representations "
            "WHERE representation_id=?",
            (representation_id,),
        ).fetchone()
        if row is None:
            raise ValueError(f"missing long-witness representation {representation_id}")
        target.execute(
            "INSERT INTO hard_case_representations VALUES (?,?,?,?,?,?,?,?,?)",
            (
                LONG_CASE_ID,
                representation_id,
                str(row[0]),
                f"representation-{representation_id}",
                -1,
                -1,
                int(row[1]),
                int(row[2]),
                str(row[3]),
            ),
        )
    long_counts = {0: 0, 1: 0}
    for left, right, lower, upper, witness_length, provenance in protected_long:
        target.execute(
            "INSERT INTO hard_case_pairs VALUES (?,?,?,?,?,?,?,?,?)",
            (
                LONG_CASE_ID,
                int(left),
                int(right),
                "cc",
                int(lower),
                int(upper),
                int(witness_length),
                "replay_exact" if int(upper) == 0 else "replay_plus_attested_exact",
                str(provenance),
            ),
        )
        long_counts[int(upper)] += 1

    protected_count = len(
        {mapped_state[0] for mapped_state in mapped.values()}
        | set(long_representation_ids)
    )
    target.execute(
        "INSERT INTO meta VALUES ('protected_representation_count',?)",
        (str(protected_count),),
    )
    source.close()

    integrity = target.execute("PRAGMA integrity_check").fetchone()[0]
    if integrity != "ok":
        raise RuntimeError(f"SQLite integrity check failed: {integrity}")
    target.execute("PRAGMA optimize")
    target.commit()
    target.close()
    os.replace(temporary, args.output)
    print(
        json.dumps(
            {
                "schema": SCHEMA,
                "output": str(args.output.resolve()),
                "representations": len(mapped),
                "exact_cc1_pairs": pair_count[1],
                "exact_cc2_pairs": pair_count[2],
                "protected_exact_long_cc0_pairs": long_counts[0],
                "protected_exact_long_cc1_pairs": long_counts[1],
                "protected_representations": protected_count,
                "bytes": args.output.stat().st_size,
                "sha256": sha256(args.output),
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
